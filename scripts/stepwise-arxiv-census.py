#!/usr/bin/env python3
"""Run or verify the serial, resumable, single-pass arXiv census."""

from __future__ import annotations

import csv
import fcntl
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
HEADER = [
    "id", "engine_status", "finalizer_status", "cold_starts", "suspensions",
    "local_step_retries", "replayed_delivered_tokens", "replayed_dispatches",
    "cumulative_fuel", "resource_wait_ns", "engine_ns", "error_cluster",
    "guard_status", "row_wall_ns", "finalizer_ns",
    "source_read_ns", "format_read_ns", "format_restore_ns", "setup_ns",
    "accepted_wall_ns", "pdf_font_resources_ns", "map_resolve_ns",
    "positioning_ns", "vf_ns", "font_usage_ns", "destinations_ns",
    "annotations_ns", "pdf_object_ns", "font_embed_ns", "serialization_ns",
    "image_import_ns", "image_parse_copy_ns", "image_decode_ns",
    "image_transform_ns", "image_encode_ns", "image_cache_hits",
    "image_pixels", "image_rows", "image_raw_bytes", "image_color_bytes",
    "image_alpha_bytes", "image_peak_row_bytes", "image_deflate_level",
    "image_deflate_window_bits",
    "validation_ns", "pdf_build_ns", "materialize_ns",
    "run_wall_ns", "images", "raster_images", "pdf_images",
    "image_input_bytes", "unique_images", "lowered_images", "objects",
    "output_bytes",
]
TELEMETRY_FIELDS = HEADER[3:11]


def env_path(name: str, default: Path | None = None) -> Path:
    value = os.environ.get(name)
    if value:
        return Path(value).resolve()
    if default is None:
        fail(f"{name} must be set")
    return default.resolve()


def env_int(name: str, default: int, maximum: int) -> int:
    raw = os.environ.get(name, str(default))
    if not raw.isdecimal() or not 1 <= int(raw) <= maximum:
        fail(f"{name} must be in 1..{maximum}")
    return int(raw)


def fail(message: str) -> None:
    raise SystemExit(message)


def sha256_file(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def sha256_tree(path: Path) -> str:
    value = hashlib.sha256()
    if not path.is_dir():
        fail(f"source directory is missing: {path}")
    for item in sorted(candidate for candidate in path.rglob("*") if candidate.is_file()):
        relative = item.relative_to(path).as_posix().encode()
        value.update(len(relative).to_bytes(8, "big"))
        value.update(relative)
        value.update(bytes.fromhex(sha256_file(item)))
    return value.hexdigest()


def atomic_json(path: Path, value: object) -> None:
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    os.replace(temporary, path)


def entrypoint(directory: Path) -> Path | None:
    declaration = re.compile(rb"^[ \t]*\\documentclass(?:[ \t]|\[|\{|$)", re.MULTILINE)
    candidates = []
    for name in ("main.tex", "manuscript.tex", "arxiv_version.tex", "paper.tex", "ms.tex"):
        path = directory / name
        if path.is_file() and declaration.search(path.read_bytes()):
            return path
    for path in directory.rglob("*.tex"):
        if re.search(r"/(supp|supplement|appendix)[^/]*\.tex$", path.as_posix()):
            continue
        if declaration.search(path.read_bytes()):
            candidates.append(path)
    return min(candidates, key=lambda path: path.as_posix()) if candidates else None


def error_cluster(log: str, status: int) -> str:
    if status == 0:
        return "none"
    if status == 124:
        return "guard-timeout-or-rss"
    patterns = (
        ("panicked at crates/tex-state/src/stores.rs", "stores-snapshot-panic"),
        ("ropbox{", "image-cropbox-filename"),
        ("invalid UTF-8", "invalid-utf8-input"),
        ("valid UTF-8", "invalid-utf8-input"),
        ("action type missing", "pdf-action-type"),
        ("End of file on the terminal", "terminal-read-eof"),
        ("invalid parameter token", "macro-parameter-token"),
        ("failed to open input", "missing-generated-input"),
        ("distribution unavailable", "missing-distribution-resource"),
        ("expansion work limit", "expansion-work-limit"),
    )
    return next((cluster for text, cluster in patterns if text in log), "other-engine-error")


def parse_telemetry(log: str) -> dict[str, int]:
    line = next((line for line in reversed(log.splitlines()) if line.startswith("RESOURCE_TELEMETRY ")), "")
    values = {name: int(value) for name, value in re.findall(r" ([a-z_]+)=([0-9]+)", line)}
    return {name: values.get(name, 0) for name in TELEMETRY_FIELDS}


def parse_phase_telemetry(log: str) -> dict[str, int]:
    values = {}
    prefixes = ("RESOURCE_STARTUP_TELEMETRY ", "RESOURCE_ENGINE_ACCEPTED ",
                "PDF_TELEMETRY ", "PDF_DRIVER_BUILD ", "PDF_DRIVER_TELEMETRY ")
    for line in log.splitlines():
        if line.startswith(prefixes):
            values.update({name: int(value) for name, value in re.findall(r" ([a-z_]+)=([0-9]+)", line)})
    values["pdf_font_resources_ns"] = values.pop("font_resources_ns", 0)
    values["pdf_object_ns"] = values.pop("object_ns", 0)
    return {name: values.get(name, 0) for name in HEADER[15:]}


def artifact_hashes(results: Path, key: str, finalizer_status: str) -> dict[str, str | None]:
    log = results / f"{key}.engine.log"
    pdf = results / f"{key}.pdf"
    inputs = results / f"{key}.inputs.tsv"
    return {
        "log_sha256": sha256_file(log),
        "pdf_sha256": sha256_file(pdf) if finalizer_status == "complete" and pdf.exists() else None,
        "inputs_sha256": sha256_file(inputs) if inputs.exists() else None,
    }


def validate_record(record: dict, results: Path, paper: str, source_sha256: str) -> None:
    if record.get("id") != paper or record.get("source_sha256") != source_sha256:
        fail(f"resumable row identity changed: {paper}")
    expected = artifact_hashes(results, paper.replace("/", "_"), record["finalizer_status"])
    if record.get("artifacts") != expected:
        fail(f"resumable row artifacts changed: {paper}")


def write_summary(results: Path, papers: list[dict], records: dict[str, dict]) -> None:
    temporary = results / "summary.tsv.tmp"
    with temporary.open("w", newline="") as output:
        writer = csv.DictWriter(output, fieldnames=HEADER, delimiter="\t", lineterminator="\n")
        writer.writeheader()
        for paper in papers:
            if paper["id"] in records:
                writer.writerow({name: records[paper["id"]][name] for name in HEADER})
    os.replace(temporary, results / "summary.tsv")


def outcome_digest(papers: list[dict], records: dict[str, dict]) -> str:
    stable = [
        [paper["id"], records[paper["id"]]["engine_status"],
         records[paper["id"]]["finalizer_status"], records[paper["id"]]["error_cluster"],
         records[paper["id"]]["guard_status"]]
        for paper in papers
    ]
    return hashlib.sha256(json.dumps(stable, separators=(",", ":")).encode()).hexdigest()


def main() -> None:
    sample = env_path("UMBER_ARXIV_SAMPLE", ROOT / "scripts/pdftex-arxiv-sample-100.tsv")
    corpus = env_path("UMBER_ARXIV_CORPUS", ROOT / "third_party/arxiv-sample-100/sources")
    format_path = env_path("UMBER_ARXIV_FORMAT")
    distribution = env_path("UMBER_ARXIV_DISTRIBUTION")
    binary = env_path("UMBER_ARXIV_BINARY", ROOT / "target/debug/umber")
    results = env_path("UMBER_ARXIV_RESULTS", ROOT / "target/stepwise-arxiv-census")
    texmf = env_path("UMBER_ARXIV_TEXMF", ROOT / "third_party/texlive-20260301-texmf/texmf-dist")
    limit = env_int("UMBER_ARXIV_LIMIT", 100, 100)
    timeout = env_int("UMBER_ARXIV_TIMEOUT_SECONDS", 120, 120)
    rss = env_int("UMBER_ARXIV_MAX_RSS_MIB", 1536, 1536)
    fuel = env_int("UMBER_ARXIV_ENGINE_FUEL", 100_000_000, 100_000_000)
    offline = os.environ.get("UMBER_ARXIV_OFFLINE", "1")
    verify_only = os.environ.get("UMBER_ARXIV_VERIFY_ONLY", "0") == "1"
    if offline not in ("0", "1"):
        fail("UMBER_ARXIV_OFFLINE must be 0 or 1")
    if os.environ.get("UMBER_ARXIV_FINALIZE", "1") != "1":
        fail("UMBER_ARXIV_FINALIZE=0 is retired; the census always records PDF finalization")
    manifest = distribution / "manifest.json"
    for path, label in ((sample, "sample"), (format_path, "format"), (manifest, "distribution manifest"), (binary, "Umber binary")):
        if not path.is_file():
            fail(f"{label} is missing: {path}")
    if not os.access(binary, os.X_OK):
        fail(f"Umber binary is not executable: {binary}")

    with sample.open(newline="") as source:
        papers = list(csv.DictReader(source, delimiter="\t"))[:limit]
    if len(papers) != limit or len({row["id"] for row in papers}) != limit:
        fail("sample does not contain the requested number of unique rows")
    source_hashes = {
        row["id"]: sha256_tree(corpus / row["id"].replace("/", "_")) for row in papers
    }
    immutable = {
        "schema": 2,
        "binary_path": str(binary),
        "binary_sha256": sha256_file(binary),
        "format_path": str(format_path),
        "format_sha256": sha256_file(format_path),
        "distribution_path": str(distribution),
        "distribution_manifest_sha256": sha256_file(manifest),
        "sample_sha256": sha256_file(sample),
        "source_sha256": source_hashes,
        "limit": limit,
        "engine_fuel": fuel,
        "max_rss_mib": rss,
        "timeout_seconds": timeout,
    }
    execution = {"immutable": immutable, "offline": offline == "1"}
    results.mkdir(parents=True, exist_ok=True)
    lock = (results / "run.lock").open("w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        fail(f"another census owns the results directory: {results}")
    (results / "rows").mkdir(exist_ok=True)
    identity_path = results / "run-identity.json"
    if identity_path.exists():
        prior = json.loads(identity_path.read_text())
        expected = immutable if verify_only else execution
        actual = prior["immutable"] if verify_only else prior
        if actual != expected:
            fail("results directory belongs to a different census identity")
    elif verify_only:
        fail("cannot verify a census without run-identity.json")
    else:
        atomic_json(identity_path, execution)

    records = {}
    for paper in papers:
        record_path = results / "rows" / f"{paper['id'].replace('/', '_')}.json"
        if record_path.exists():
            record = json.loads(record_path.read_text())
            validate_record(record, results, paper["id"], source_hashes[paper["id"]])
            records[paper["id"]] = record
    write_summary(results, papers, records)

    if verify_only:
        if offline != "1":
            fail("verification must be requested with UMBER_ARXIV_OFFLINE=1")
        if len(records) != limit:
            fail(f"cannot verify incomplete census: {len(records)}/{limit} rows")
        receipt = {
            "schema": 1,
            "offline": True,
            "verified_rows": limit,
            "outcome_sha256": outcome_digest(papers, records),
            "run_identity_sha256": sha256_file(identity_path),
            "children_launched": 0,
            "basis": "authenticated resources are cached before use; immutable inputs and completed artifacts were rehashed",
        }
        atomic_json(results / "offline-verification.json", receipt)
        print(f"stepwise arXiv census offline verification: {results / 'offline-verification.json'}")
        return

    guard = ROOT / "scripts/run-umber-guarded.py"
    run_flags = ["--pdflatex", "--distribution", str(distribution), "--format", str(format_path)]
    if offline == "1":
        run_flags.append("--offline")
    texinputs = f"{texmf}/tex/latex//:{texmf}/tex/generic//:{texmf}/tex/plain//:"
    texfonts = f"{texmf}/fonts/tfm//:"
    for paper in papers:
        paper_id = paper["id"]
        if paper_id in records:
            continue
        key = paper_id.replace("/", "_")
        source_dir = corpus / key
        main_input = entrypoint(source_dir)
        row = {name: 0 for name in HEADER}
        row["id"] = paper_id
        row["source_sha256"] = source_hashes[paper_id]
        if main_input is None:
            row.update(engine_status="no-entrypoint", finalizer_status="not-run", error_cluster="no-entrypoint")
            log_path = results / f"{key}.engine.log"
            log_path.write_text("no live document entrypoint\n")
        else:
            log_path = results / f"{key}.engine.log"
            partial_log = results / f"{key}.engine.log.partial"
            partial_pdf = results / f"{key}.pdf.partial"
            partial_inputs = results / f"{key}.inputs.tsv.partial"
            env = os.environ.copy()
            env.update(UMBER_RESOURCE_TELEMETRY="1", UMBER_ENGINE_FUEL=str(fuel),
                       TEXINPUTS=f"{main_input.parent}:{texinputs}", TEXFONTS=texfonts)
            command = [sys.executable, str(guard), "--timeout-seconds", str(timeout),
                       "--max-rss-mib", str(rss), "--term-grace-seconds", "2", "--",
                       str(binary), "run", *run_flags, "--pdf", str(partial_pdf),
                       "--input-records-out", str(partial_inputs), str(main_input)]
            started = time.monotonic_ns()
            with partial_log.open("wb") as output:
                completed = subprocess.run(command, cwd=results, env=env, stdout=output, stderr=subprocess.STDOUT)
            wall_ns = time.monotonic_ns() - started
            os.replace(partial_log, log_path)
            log = log_path.read_text(errors="replace")
            accepted = "RESOURCE_ENGINE_ACCEPTED" in log
            telemetry = parse_telemetry(log)
            row.update(telemetry)
            row.update(parse_phase_telemetry(log))
            row["guard_status"] = completed.returncode
            row["row_wall_ns"] = wall_ns
            row["finalizer_ns"] = max(0, wall_ns - telemetry["engine_ns"] - telemetry["resource_wait_ns"]) if accepted else 0
            row["engine_status"] = "accepted" if accepted else ("guard-timeout-or-rss" if completed.returncode == 124 else "failed")
            row["finalizer_status"] = ("complete" if completed.returncode == 0 else
                                       "guard-timeout-or-rss" if accepted and completed.returncode == 124 else
                                       "failed" if accepted else "not-run")
            row["error_cluster"] = error_cluster(log, completed.returncode)
            if completed.returncode == 0:
                if partial_pdf.exists():
                    os.replace(partial_pdf, results / f"{key}.pdf")
                if partial_inputs.exists():
                    os.replace(partial_inputs, results / f"{key}.inputs.tsv")
        row["artifacts"] = artifact_hashes(results, key, row["finalizer_status"])
        atomic_json(results / "rows" / f"{key}.json", row)
        records[paper_id] = row
        write_summary(results, papers, records)

    print(f"stepwise arXiv census: {results / 'summary.tsv'}")


if __name__ == "__main__":
    main()
