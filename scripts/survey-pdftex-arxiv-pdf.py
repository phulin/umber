#!/usr/bin/env python3
"""Run or verify the pinned pdfTeX PDF-success survey for the arXiv corpus."""

from __future__ import annotations

import argparse
import concurrent.futures
import csv
import hashlib
import json
import os
import re
import resource
import shutil
import subprocess
import sys
from pathlib import Path

from arxiv_corpus import (
    archive_file_bytes,
    materialize,
    sha256_file,
    source_identity,
    source_jobname,
    verify_view,
)
from texlive import verify_runtime_tree


SCHEMA = 1
TERM_GRACE_SECONDS = 2
SOURCE_DATE_EPOCH = "1772323200"


def fail(message: str) -> None:
    raise SystemExit(message)


def canonical_json(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def write_json(path: Path, value: object) -> None:
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    os.replace(temporary, path)


def file_identity(path: Path, relative_to: Path | None = None) -> dict[str, object]:
    recorded = path.relative_to(relative_to).as_posix() if relative_to else str(path)
    return {
        "path": recorded,
        "bytes": path.stat().st_size,
        "sha256": sha256_file(path),
    }


def read_lock(path: Path, expected_rows: int) -> list[dict[str, object]]:
    with path.open(newline="") as source:
        rows = list(csv.DictReader(source, delimiter="\t"))
    ids = [row["id"] for row in rows]
    if len(rows) != expected_rows or len(set(ids)) != expected_rows:
        fail(f"source lock is not the expected unique {expected_rows}-row sample")
    required = {
        "id", "source_sha256", "source_bytes", "first_submitted",
        "shuffle_sha256", "entrypoint",
    }
    if not rows or set(rows[0]) != required:
        fail("source lock has an unexpected schema")
    for order, row in enumerate(rows, 1):
        row["lock_order"] = order
        row["source_bytes"] = int(row["source_bytes"])
    return rows


def declared_compiler(archive: Path) -> tuple[str, dict[str, object]]:
    files = archive_file_bytes(archive)
    raw = files.get("00README.json")
    if raw is None:
        fail(f"archive has no 00README.json compiler declaration: {archive}")
    try:
        manifest = json.loads(raw)
        compiler = manifest["process"]["compiler"]
    except (json.JSONDecodeError, KeyError, TypeError) as error:
        fail(f"archive has an invalid compiler declaration: {archive}: {error}")
    if compiler not in ("latex", "pdflatex", "xelatex"):
        fail(f"archive has an unsupported compiler declaration: {archive}: {compiler!r}")
    return compiler, {
        "path": "00README.json",
        "bytes": len(raw),
        "sha256": hashlib.sha256(raw).hexdigest(),
    }


def inspect_sources(
    rows: list[dict[str, object]], archives: Path
) -> tuple[list[dict[str, object]], dict[str, int]]:
    selected = []
    counts: dict[str, int] = {}
    for row in rows:
        row_id = str(row["id"])
        archive = archives / f"{row_id.replace('/', '_')}.src"
        if not archive.is_file():
            fail(f"source archive is missing: {archive}")
        if archive.stat().st_size != row["source_bytes"]:
            fail(f"{row_id}: archive byte length differs from lock")
        if sha256_file(archive) != row["source_sha256"]:
            fail(f"{row_id}: archive SHA-256 differs from lock")
        compiler, declaration = declared_compiler(archive)
        counts[compiler] = counts.get(compiler, 0) + 1
        if compiler != "pdflatex":
            continue
        identity = source_identity(archive, str(row["entrypoint"]))
        entrypoint = archive_file_bytes(archive).get(str(row["entrypoint"]))
        if entrypoint is None:
            fail(f"{row_id}: locked entrypoint is absent from the archive")
        selected.append({
            **row,
            "archive": archive,
            "declared_reference_engine": compiler,
            "declared_engine_manifest": declaration,
            "source_identity": {
                **identity,
                "archive_bytes": archive.stat().st_size,
                "entrypoint_bytes": len(entrypoint),
                "entrypoint_sha256": hashlib.sha256(entrypoint).hexdigest(),
            },
        })
    return selected, counts


def validate_authority(arguments: argparse.Namespace) -> dict[str, object]:
    required = (
        (arguments.oracle, "oracle binary"),
        (arguments.oracle_build_record, "oracle build record"),
        (arguments.format, "reference format"),
        (arguments.format_receipt, "reference format receipt"),
        (arguments.runtime_lock, "runtime lock"),
    )
    for path, label in required:
        if not path.is_file():
            fail(f"{label} is missing: {path}")
    if not os.access(arguments.oracle, os.X_OK):
        fail(f"oracle binary is not executable: {arguments.oracle}")
    if not arguments.runtime_root.is_dir():
        fail(f"runtime tree is missing: {arguments.runtime_root}")

    oracle = file_identity(arguments.oracle)
    build_record = file_identity(arguments.oracle_build_record)
    clean_record = f"executable clean target/pdftex14029-oracle/bin/umber-pdftex14029-oracle-clean {oracle['sha256']}"
    if clean_record not in arguments.oracle_build_record.read_text(errors="replace").splitlines():
        fail("oracle binary identity is absent from its clean build record")
    receipt = json.loads(arguments.format_receipt.read_text())
    format_identity = file_identity(arguments.format)
    if receipt.get("engine", {}).get("sha256") != oracle["sha256"]:
        fail("reference format was not built by the selected clean oracle")
    if receipt.get("format", {}).get("sha256") != format_identity["sha256"]:
        fail("reference format differs from its receipt")
    distribution, tree_ahash64 = verify_runtime_tree(
        arguments.runtime_root, arguments.runtime_lock
    )
    return {
        "binary": oracle,
        "build_record": build_record,
        "format": format_identity,
        "format_receipt": file_identity(arguments.format_receipt),
        "runtime": {
            "path": str(arguments.runtime_root),
            "distribution": distribution,
            "tree_ahash64": tree_ahash64,
            "lock": file_identity(arguments.runtime_lock),
        },
    }


def row_authority(authority: dict[str, object]) -> dict[str, object]:
    binary = authority["binary"]
    build_record = authority["build_record"]
    format_identity = authority["format"]
    format_receipt = authority["format_receipt"]
    runtime = authority["runtime"]
    return {
        "binary_sha256": binary["sha256"],
        "build_record_sha256": build_record["sha256"],
        "format_sha256": format_identity["sha256"],
        "format_receipt_sha256": format_receipt["sha256"],
        "runtime_distribution": runtime["distribution"],
        "runtime_tree_ahash64": runtime["tree_ahash64"],
        "runtime_lock_sha256": runtime["lock"]["sha256"],
    }


def first_failure(log: str, stdout: str, stderr: str, status: int) -> dict[str, object]:
    if status == 124:
        return {"kind": "guard-timeout", "channel": "process", "line": "timeout guard expired", "context": []}
    for channel, text in (("log", log), ("stdout", stdout), ("stderr", stderr)):
        lines = text.splitlines()
        for index, line in enumerate(lines):
            if not line.startswith("!"):
                continue
            context = []
            for following in lines[index + 1:index + 7]:
                if not following or following.startswith("!"):
                    break
                context.append(following)
            if line.startswith("! LaTeX Error:"):
                kind = "latex-error"
            elif re.match(r"! Package .+ Error:", line):
                kind = "package-error"
            elif "pdfTeX error" in line:
                kind = "pdftex-error"
            elif line == "! Undefined control sequence.":
                kind = "undefined-control-sequence"
            else:
                kind = "tex-error"
            return {"kind": kind, "channel": channel, "line": line, "context": context}
    return {
        "kind": "process-exit-without-tex-diagnostic",
        "channel": "process",
        "line": f"oracle exited with status {status}",
        "context": [],
    }


def address_space_limiter(max_rss_mib: int):
    def apply_limit() -> None:
        limit = max_rss_mib * 1024 * 1024
        resource.setrlimit(resource.RLIMIT_AS, (limit, limit))
    return apply_limit


def run_row(
    row: dict[str, object], arguments: argparse.Namespace,
    authority: dict[str, object],
) -> dict[str, object]:
    row_id = str(row["id"])
    row_dir = arguments.results / "rows" / row_id.replace("/", "_")
    if row_dir.exists():
        fail(f"incomplete row directory already exists: {row_dir}")
    row_dir.mkdir(parents=True)
    source = row_dir / "source"
    run = row_dir / "run"
    materialize(row["archive"], source)
    verify_view(row["archive"], source)
    shutil.copytree(source, run)
    for name in ("texmf-var", "texmf-config", "texmf-home", "texmf-sysvar", "texmf-sysconfig"):
        (run / name).mkdir()

    entrypoint = str(row["entrypoint"])
    jobname = source_jobname(entrypoint)
    stdout_path = row_dir / "oracle.stdout"
    stderr_path = row_dir / "oracle.stderr"
    time_path = row_dir / "oracle.time"
    command = [
        "/usr/bin/time", "-v", "-o", str(time_path),
        "timeout", "-k", f"{TERM_GRACE_SECONDS}s", f"{arguments.timeout_seconds}s",
        str(arguments.oracle), f"--fmt={arguments.format}", "--output-format=pdf",
        "--interaction=nonstopmode", "--halt-on-error", entrypoint,
    ]
    environment = os.environ.copy()
    environment.update({
        "TEXMFCNF": str(arguments.runtime_root / "web2c"),
        "TEXMFROOT": str(arguments.runtime_root.parent),
        "TEXMFDIST": str(arguments.runtime_root),
        "TEXMFVAR": str(run / "texmf-var"),
        "TEXMFCONFIG": str(run / "texmf-config"),
        "TEXMFHOME": str(run / "texmf-home"),
        "TEXMFSYSVAR": str(run / "texmf-sysvar"),
        "TEXMFSYSCONFIG": str(run / "texmf-sysconfig"),
        "SOURCE_DATE_EPOCH": SOURCE_DATE_EPOCH,
        "FORCE_SOURCE_DATE": "1",
    })
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        completed = subprocess.run(
            command, cwd=run, env=environment, stdout=stdout, stderr=stderr,
            check=False, preexec_fn=address_space_limiter(arguments.max_rss_mib),
        )

    log_path = run / f"{jobname}.log"
    pdf_path = run / f"{jobname}.pdf"
    log_text = log_path.read_text(errors="replace") if log_path.exists() else ""
    stdout_text = stdout_path.read_text(errors="replace")
    stderr_text = stderr_path.read_text(errors="replace")
    completion = re.search(
        rf"Output written on {re.escape(jobname)}\.pdf \((\d+) pages?, (\d+) bytes\)\.",
        log_text,
    )
    # TeX wraps long completion lines at max_print_line, so a successful exit
    # plus the source-jobname PDF is the authoritative production criterion.
    # The completion record is used only for optional page-count evidence.
    success = completed.returncode == 0 and pdf_path.is_file()
    pdf = file_identity(pdf_path, arguments.results) if pdf_path.is_file() else None
    if pdf is not None and completion is not None:
        pdf["pages"] = int(completion.group(1))
        pdf["reported_bytes"] = int(completion.group(2))
    failure = None if success else first_failure(log_text, stdout_text, stderr_text, completed.returncode)
    if not success and completed.returncode == 0:
        failure = {
            "kind": "missing-authoritative-pdf",
            "channel": "process",
            "line": "oracle exited successfully without a PDF completion record",
            "context": [],
        }
    result = {
        "schema": SCHEMA,
        "lock_order": row["lock_order"],
        "id": row_id,
        "declared_reference_engine": "pdflatex",
        "declared_engine_manifest": row["declared_engine_manifest"],
        "source": row["source_identity"],
        "oracle": row_authority(authority),
        "command": command,
        "working_directory": (Path("rows") / row_id.replace("/", "_") / "run").as_posix(),
        "jobname": jobname,
        "terminal_status": {
            "exit_status": completed.returncode,
            "classification": "PDF-success" if success else "PDF-failure",
        },
        "pdf": pdf if success else None,
        "partial_pdf": pdf if not success else None,
        "failure": failure,
        "artifacts": {
            "log": file_identity(log_path, arguments.results) if log_path.exists() else None,
            "stdout": file_identity(stdout_path, arguments.results),
            "stderr": file_identity(stderr_path, arguments.results),
            "time": file_identity(time_path, arguments.results),
        },
    }
    write_json(row_dir / "result.json", result)
    return result


def summary_for(results: list[dict[str, object]], expected_rows: int) -> dict[str, object]:
    statuses: dict[str, int] = {}
    failures: dict[str, int] = {}
    for row in results:
        classification = str(row["terminal_status"]["classification"])
        statuses[classification] = statuses.get(classification, 0) + 1
        if row["failure"] is not None:
            kind = str(row["failure"]["kind"])
            failures[kind] = failures.get(kind, 0) + 1
    return {
        "schema": SCHEMA,
        "rows": len(results),
        "unique_rows": len({row["id"] for row in results}),
        "expected_declared_pdflatex_rows": expected_rows,
        "counts": dict(sorted(statuses.items())),
        "failure_kinds": dict(sorted(failures.items())),
        "totals_reconcile": len(results) == expected_rows == sum(statuses.values()),
        "umber_invocations": 0,
        "umber_pdf_inspections": 0,
    }


def metadata_for(
    arguments: argparse.Namespace, rows: list[dict[str, object]],
    compiler_counts: dict[str, int], authority: dict[str, object],
) -> dict[str, object]:
    return {
        "schema": SCHEMA,
        "issue": "umber2-sdpz.227",
        "scope": {
            "sample_rows": arguments.expected_sample_rows,
            "declared_compilers": dict(sorted(compiler_counts.items())),
            "declared_pdflatex_rows": len(rows),
            "selection": "00README.json process.compiler == pdflatex",
        },
        "source_lock": file_identity(arguments.source_lock),
        "source_identities": {str(row["id"]): row["source_identity"] for row in rows},
        "authority": authority,
        "execution": {
            "source_date_epoch": int(SOURCE_DATE_EPOCH),
            "force_source_date": 1,
            "timeout_seconds": arguments.timeout_seconds,
            "max_address_space_mib": arguments.max_rss_mib,
            "term_grace_seconds": TERM_GRACE_SECONDS,
            "parallel_workers": arguments.workers,
            "jobname_policy": "entrypoint basename without .tex; no --jobname argument",
            "working_directory_policy": "fresh exact archive root copied to the issue-local row run directory",
            "side_file_policy": "archive side files preserved; generated side files remain in the row run directory",
        },
        "prohibitions": {
            "umber_invocations": 0,
            "umber_pdf_inspections": 0,
            "paper_patches": 0,
            "shared_state_mutations": 0,
        },
    }


def check_identity(record: dict[str, object] | None, results: Path) -> None:
    if record is None:
        return
    path = results / str(record["path"])
    actual = file_identity(path, results) if path.is_file() else None
    expected = {name: record[name] for name in ("path", "bytes", "sha256")}
    if actual != expected:
        fail(f"survey artifact identity changed: {path}")


def verify_results(
    arguments: argparse.Namespace, selected: list[dict[str, object]],
    metadata: dict[str, object],
) -> tuple[list[dict[str, object]], dict[str, object]]:
    metadata_path = arguments.results / "metadata.json"
    if not metadata_path.is_file() or json.loads(metadata_path.read_text()) != metadata:
        fail("survey metadata differs from authenticated inputs")
    rows = []
    authority = row_authority(metadata["authority"])
    for expected in selected:
        row_id = str(expected["id"])
        row_dir = arguments.results / "rows" / row_id.replace("/", "_")
        result_path = row_dir / "result.json"
        if not result_path.is_file():
            fail(f"survey row is incomplete: {row_id}")
        row = json.loads(result_path.read_text())
        if row.get("id") != row_id or row.get("lock_order") != expected["lock_order"]:
            fail(f"survey row identity differs: {row_id}")
        if row.get("source") != expected["source_identity"] or row.get("oracle") != authority:
            fail(f"survey row source/oracle identity differs: {row_id}")
        if row.get("declared_reference_engine") != "pdflatex":
            fail(f"survey row is not declared pdfLaTeX: {row_id}")
        if row.get("jobname") != source_jobname(str(expected["entrypoint"])):
            fail(f"survey row jobname differs: {row_id}")
        command = row.get("command", [])
        if any(str(item).startswith("--jobname") for item in command):
            fail(f"survey row overrides the source-derived jobname: {row_id}")
        if "--output-format=pdf" not in command or command[-1] != expected["entrypoint"]:
            fail(f"survey row command differs: {row_id}")
        expected_cwd = (Path("rows") / row_id.replace("/", "_") / "run").as_posix()
        if row.get("working_directory") != expected_cwd:
            fail(f"survey row working directory differs: {row_id}")
        verify_view(expected["archive"], row_dir / "source")
        for artifact in row["artifacts"].values():
            check_identity(artifact, arguments.results)
        check_identity(row.get("pdf") or row.get("partial_pdf"), arguments.results)
        classification = row["terminal_status"]["classification"]
        produced_pdf = row.get("pdf") is not None or row.get("partial_pdf") is not None
        expected_classification = (
            "PDF-success"
            if row["terminal_status"]["exit_status"] == 0 and produced_pdf
            else "PDF-failure"
        )
        if classification != expected_classification:
            fail(f"survey row outcome differs from its process/PDF evidence: {row_id}")
        if classification == "PDF-success":
            if row["terminal_status"]["exit_status"] != 0 or row["pdf"] is None or row["failure"] is not None:
                fail(f"invalid successful survey row: {row_id}")
        elif classification == "PDF-failure":
            if row["pdf"] is not None or row["failure"] is None:
                fail(f"invalid failed survey row: {row_id}")
        else:
            fail(f"unknown survey classification: {row_id}: {classification}")
        rows.append(row)
    if len({row["id"] for row in rows}) != len(selected):
        fail("survey does not account for each declared-pdfLaTeX row exactly once")
    results_bytes = b"".join(canonical_json(row) for row in rows)
    results_path = arguments.results / "results.jsonl"
    if not results_path.is_file() or results_path.read_bytes() != results_bytes:
        fail("results.jsonl is not the deterministic row-report reproduction")
    summary = summary_for(rows, arguments.expected_pdflatex_rows)
    summary_path = arguments.results / "summary.json"
    if not summary_path.is_file() or json.loads(summary_path.read_text()) != summary:
        fail("summary.json is not the deterministic totals reproduction")
    if not summary["totals_reconcile"]:
        fail("survey totals do not reconcile")
    return rows, summary


def publish_report(arguments: argparse.Namespace) -> None:
    if arguments.report_dir is None:
        return
    arguments.report_dir.mkdir(parents=True, exist_ok=True)
    for name in ("metadata.json", "results.jsonl", "summary.json", "verification.json"):
        shutil.copyfile(arguments.results / name, arguments.report_dir / name)


def parse_arguments() -> argparse.Namespace:
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-lock", type=Path, default=root / "scripts/pdftex-arxiv-recent-sample-100.lock.tsv")
    parser.add_argument("--archives", type=Path, required=True)
    parser.add_argument("--oracle", type=Path, required=True)
    parser.add_argument("--oracle-build-record", type=Path, required=True)
    parser.add_argument("--format", type=Path, required=True)
    parser.add_argument("--format-receipt", type=Path, required=True)
    parser.add_argument("--runtime-root", type=Path, required=True)
    parser.add_argument("--runtime-lock", type=Path, default=root / "tests/texlive-snapshot.lock")
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--report-dir", type=Path)
    parser.add_argument("--expected-sample-rows", type=int, default=100)
    parser.add_argument("--expected-pdflatex-rows", type=int, default=94)
    parser.add_argument("--timeout-seconds", type=int, default=120)
    parser.add_argument("--max-rss-mib", type=int, default=1536)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--verify-only", action="store_true")
    arguments = parser.parse_args()
    for name in ("source_lock", "archives", "oracle", "oracle_build_record", "format", "format_receipt", "runtime_root", "runtime_lock", "results", "report_dir"):
        value = getattr(arguments, name)
        if value is not None:
            setattr(arguments, name, value.resolve())
    if arguments.expected_sample_rows < 1 or arguments.expected_pdflatex_rows < 1:
        fail("expected row counts must be positive")
    if arguments.timeout_seconds < 1 or arguments.max_rss_mib < 1 or arguments.workers < 1:
        fail("guard and worker limits must be positive")
    return arguments


def main() -> None:
    arguments = parse_arguments()
    rows = read_lock(arguments.source_lock, arguments.expected_sample_rows)
    selected, compiler_counts = inspect_sources(rows, arguments.archives)
    if len(selected) != arguments.expected_pdflatex_rows:
        fail(
            f"declared-pdfLaTeX denominator differs: "
            f"{len(selected)} != {arguments.expected_pdflatex_rows}"
        )
    authority = validate_authority(arguments)
    metadata = metadata_for(arguments, selected, compiler_counts, authority)
    if arguments.verify_only:
        verified, summary = verify_results(arguments, selected, metadata)
        receipt = {
            "schema": SCHEMA,
            "mode": "verify-only",
            "compilers_launched": 0,
            "verified_rows": len(verified),
            "results_sha256": sha256_file(arguments.results / "results.jsonl"),
            "summary_sha256": sha256_file(arguments.results / "summary.json"),
            "totals_reconcile": summary["totals_reconcile"],
        }
        write_json(arguments.results / "verification.json", receipt)
        publish_report(arguments)
        print(json.dumps(receipt, sort_keys=True))
        return

    arguments.results.mkdir(parents=True, exist_ok=True)
    (arguments.results / "rows").mkdir(exist_ok=True)
    metadata_path = arguments.results / "metadata.json"
    if metadata_path.exists():
        if json.loads(metadata_path.read_text()) != metadata:
            fail("results directory belongs to a different survey identity")
    else:
        write_json(metadata_path, metadata)
    existing = {}
    for row in selected:
        row_id = str(row["id"])
        result_path = arguments.results / "rows" / row_id.replace("/", "_") / "result.json"
        if result_path.is_file():
            existing[row_id] = json.loads(result_path.read_text())
    pending = [row for row in selected if str(row["id"]) not in existing]
    with concurrent.futures.ThreadPoolExecutor(max_workers=arguments.workers) as pool:
        completed = list(pool.map(lambda row: run_row(row, arguments, authority), pending))
    by_id = {**existing, **{str(row["id"]): row for row in completed}}
    ordered = [by_id[str(row["id"])] for row in selected]
    (arguments.results / "results.jsonl").write_bytes(
        b"".join(canonical_json(row) for row in ordered)
    )
    write_json(arguments.results / "summary.json", summary_for(ordered, arguments.expected_pdflatex_rows))
    print(json.dumps(summary_for(ordered, arguments.expected_pdflatex_rows), sort_keys=True))


if __name__ == "__main__":
    main()
