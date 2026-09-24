#!/usr/bin/env python3
"""Qualify the locked arXiv PDF survey in DVI and compare full sources serially."""

from __future__ import annotations

import argparse
import fcntl
import importlib.util
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from types import SimpleNamespace

from arxiv_corpus import materialize, sha256_file, source_jobname, verify_view
from texlive import ahash64_file


ROOT = Path(__file__).resolve().parent.parent
SURVEY_PATH = Path(__file__).with_name("survey-pdftex-arxiv-pdf.py")
spec = importlib.util.spec_from_file_location("arxiv_pdf_survey", SURVEY_PATH)
assert spec is not None and spec.loader is not None
survey = importlib.util.module_from_spec(spec)
spec.loader.exec_module(survey)


def fail(message: str) -> None:
    raise SystemExit(message)


def identity(path: Path) -> dict[str, object]:
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256_file(path)}


def write_json(path: Path, value: object) -> None:
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    os.replace(temporary, path)


def dvi_pages(path: Path) -> int:
    data = path.read_bytes()
    trailer = len(data) - 1
    while trailer >= 0 and data[trailer] == 223:
        trailer -= 1
    if trailer < 5 or data[trailer - 5] != 249:
        fail(f"DVI has no post_post trailer: {path}")
    post = int.from_bytes(data[trailer - 4:trailer], "big")
    if post + 29 > len(data) or data[post] != 248:
        fail(f"DVI has an invalid postamble pointer: {path}")
    return int.from_bytes(data[post + 27:post + 29], "big")


def artifact(path: Path | None) -> dict[str, object] | None:
    return identity(path) if path is not None and path.is_file() else None


def check_artifact(record: dict | None, results: Path) -> None:
    if record is None:
        return
    path = Path(str(record["path"]))
    if not path.is_relative_to(results) or not path.is_file() or identity(path) != record:
        fail(f"cohort artifact changed: {path}")


def check_record(result: dict, source: dict, pdf: dict, args: argparse.Namespace) -> None:
    row_id = str(source["id"])
    if (result.get("id") != row_id or result.get("lock_order") != source["lock_order"]
            or result.get("source") != source["source_identity"]
            or result.get("entrypoint") != source["entrypoint"]
            or result.get("jobname") != source_jobname(str(source["entrypoint"]))
            or result.get("pdf") != pdf.get("pdf")
            or result.get("pdf_pages") != (pdf.get("pdf") or {}).get("pages")):
        fail(f"cohort row identity differs: {row_id}")
    if pdf["terminal_status"]["classification"] != "PDF-success":
        if result.get("status") != "PDF-ineligible" or "reference" in result or "umber" in result:
            fail(f"invalid PDF-ineligible cohort row: {row_id}")
        return
    reference = result.get("reference")
    if not isinstance(reference, dict):
        fail(f"reference DVI qualification missing: {row_id}")
    command = reference.get("command", [])
    if ("--output-format=dvi" not in command or "-progname=pdflatex-dev" not in command
            or str(args.oracle) not in command
            or f"--fmt={args.reference_format}" not in command
            or command[-1] != source["entrypoint"]
            or any(str(item).startswith("--jobname") for item in command)):
        fail(f"reference DVI command changed: {row_id}")
    row_dir = args.results / "rows" / row_id.replace("/", "_")
    if reference.get("working_directory") != str(row_dir / "reference"):
        fail(f"reference DVI working directory changed: {row_id}")
    for item in (reference.get("dvi"), *(reference.get("artifacts") or {}).values()):
        check_artifact(item, args.results)
    if reference["status"] != "DVI-success":
        if (result.get("status") != "DVI-ineligible" or "umber" in result
                or reference["exit_status"] == 0 and reference.get("dvi") is not None):
            fail(f"invalid DVI-ineligible cohort row: {row_id}")
        return
    dvi = reference.get("dvi")
    if (reference["exit_status"] != 0 or dvi is None
            or reference.get("pages") != dvi_pages(Path(str(dvi["path"])))):
        fail(f"invalid successful DVI qualification: {row_id}")
    umber = result.get("umber")
    if not isinstance(umber, dict):
        fail(f"Umber DVI result missing: {row_id}")
    command = umber.get("command", [])
    required = ("--pdflatex", "--distribution", str(args.distribution),
                "--distribution-ahash64", args.distribution_ahash64,
                "--format", str(args.umber_format), "--offline",
                "--expansion-fuel", "500000000", "--execution-steps", "10000000", "--dvi")
    if (not all(item in command for item in required) or command[-1] != source["entrypoint"]
            or "--pdf" in command or any(str(item).startswith("--jobname") for item in command)
            or umber.get("working_directory") != str(row_dir / "umber")):
        fail(f"Umber DVI command changed: {row_id}")
    for item in (umber.get("dvi"), *(umber.get("artifacts") or {}).values(),
                 umber.get("comparison_log")):
        check_artifact(item, args.results)
    if umber["exit_status"] != 0 or umber.get("dvi") is None:
        expected_status = "Umber-failure"
    else:
        if umber.get("pages") != dvi_pages(Path(str(umber["dvi"]["path"]))):
            fail(f"Umber DVI page count changed: {row_id}")
        compare = umber.get("comparison_command", [])
        if (compare[:3] != [str(args.parity_harness), "--compare-existing-dvi", str(dvi["path"])]
                or len(compare) != 8 or compare[3] != umber["dvi"]["path"]
                or compare[4:6] != ["--label", row_id]
                or compare[6:8] != ["--triage-dir", str(row_dir / "triage")]):
            fail(f"DVI comparison command changed: {row_id}")
        comparison_log = umber.get("comparison_log")
        if comparison_log is None:
            fail(f"DVI comparison log missing: {row_id}")
        comparison_text = Path(str(comparison_log["path"])).read_text(errors="replace")
        if umber.get("comparison_exit_status") == 0:
            expected_status = "DVI-exact"
        elif (umber.get("comparison_exit_status") == 1
              and re.search(r" DVI mismatch at byte \d+ on page ", comparison_text)):
            expected_status = "DVI-diverged"
        else:
            expected_status = "comparison-error"
    if result.get("status") != expected_status or umber.get("status") != expected_status:
        fail(f"cohort status differs from process evidence: {row_id}")


def verify_pdf_survey(args: argparse.Namespace) -> tuple[list[dict], dict]:
    metadata_path = args.pdf_survey / "metadata.json"
    if not metadata_path.is_file():
        fail(f"completed PDF survey is missing: {metadata_path}")
    metadata = json.loads(metadata_path.read_text())
    source_lock = metadata["source_lock"]
    if identity(args.source_lock) != source_lock:
        fail("PDF survey source lock differs from selected corpus")
    scope = metadata["scope"]
    rows = survey.read_lock(args.source_lock, scope["sample_rows"])
    selected, counts = survey.inspect_sources(rows, args.archives)
    if counts != scope["declared_compilers"] or len(selected) != scope["declared_pdflatex_rows"]:
        fail("PDF survey declared-compiler denominator changed")
    authority_args = SimpleNamespace(
        oracle=args.oracle, oracle_build_record=args.oracle_build_record,
        format=args.reference_format, format_receipt=args.format_receipt,
        runtime_root=args.runtime_root, runtime_lock=args.runtime_lock,
    )
    authority = survey.validate_authority(authority_args)
    if metadata["authority"] != authority:
        fail("PDF survey oracle, format, or runtime identity changed")
    if metadata["source_identities"] != {str(row["id"]): row["source_identity"] for row in selected}:
        fail("PDF survey source identities changed")
    survey_args = SimpleNamespace(results=args.pdf_survey, expected_pdflatex_rows=len(selected))
    pdf_rows, summary = survey.verify_results(survey_args, selected, metadata)
    if not summary["totals_reconcile"]:
        fail("PDF survey is incomplete")
    return list(zip(selected, pdf_rows)), metadata


def run_reference(args: argparse.Namespace, row: dict, row_dir: Path) -> dict:
    run = row_dir / "reference"
    materialize(Path(row["archive"]), run)
    verify_view(Path(row["archive"]), run)
    entrypoint = str(row["entrypoint"])
    environment = survey.reference_environment(args.runtime_root, run, args.reference_format)
    jobname = source_jobname(entrypoint)
    command = ["/usr/bin/time", "-v", "-o", str(row_dir / "reference.time"),
               "timeout", "-k", "2s", f"{args.timeout_seconds}s",
               str(args.oracle), "-progname=pdflatex-dev",
               f"--fmt={args.reference_format}", "--output-format=dvi",
               "--interaction=nonstopmode", "--halt-on-error", entrypoint]
    stdout = row_dir / "reference.stdout"
    stderr = row_dir / "reference.stderr"
    with stdout.open("wb") as out, stderr.open("wb") as err:
        completed = subprocess.run(
            command, cwd=run, env=environment, stdout=out, stderr=err,
            check=False, preexec_fn=survey.address_space_limiter(args.max_rss_mib),
        )
    log = run / f"{jobname}.log"
    dvi = run / f"{jobname}.dvi"
    success = completed.returncode == 0 and dvi.is_file()
    return {
        "command": command, "working_directory": str(run), "exit_status": completed.returncode,
        "status": "DVI-success" if success else "DVI-failure",
        "failure": None if success else survey.first_failure(
            log.read_text(errors="replace") if log.exists() else "",
            stdout.read_text(errors="replace"), stderr.read_text(errors="replace"),
            completed.returncode,
        ),
        "dvi": artifact(dvi), "pages": dvi_pages(dvi) if success else None,
        "artifacts": {"log": artifact(log), "stdout": artifact(stdout),
                      "stderr": artifact(stderr), "time": artifact(row_dir / "reference.time")},
    }


def run_umber(args: argparse.Namespace, row: dict, row_dir: Path, expected_dvi: Path) -> dict:
    run = row_dir / "umber"
    materialize(Path(row["archive"]), run)
    verify_view(Path(row["archive"]), run)
    entrypoint = str(row["entrypoint"])
    jobname = source_jobname(entrypoint)
    output = run / f"{jobname}.dvi"
    log = row_dir / "umber.log"
    comparison = row_dir / "comparison.log"
    environment = os.environ.copy()
    environment.update({
        "SOURCE_DATE_EPOCH": survey.SOURCE_DATE_EPOCH, "FORCE_SOURCE_DATE": "1",
        "TEXINPUTS": f"{run}:{args.runtime_root}/tex/latex-dev//:{args.runtime_root}/tex/latex//:{args.runtime_root}/tex/generic//:{args.runtime_root}/tex/plain//",
        "TEXFONTS": f"{args.runtime_root}/fonts/tfm//",
    })
    command = [sys.executable, str(ROOT / "scripts/run-umber-guarded.py"),
               "--timeout-seconds", str(args.timeout_seconds),
               "--max-rss-mib", str(args.max_rss_mib), "--term-grace-seconds", "2", "--",
               str(args.umber), "run", "--pdflatex", "--distribution", str(args.distribution),
               "--distribution-ahash64", args.distribution_ahash64,
               "--format", str(args.umber_format), "--offline",
               "--expansion-fuel", "500000000", "--execution-steps", "10000000",
               "--dvi", str(output), entrypoint]
    with log.open("wb") as out:
        completed = subprocess.run(command, cwd=run, env=environment,
                                   stdout=out, stderr=subprocess.STDOUT, check=False)
    result = {
        "command": command, "working_directory": str(run), "exit_status": completed.returncode,
        "dvi": artifact(output), "pages": dvi_pages(output) if completed.returncode == 0 and output.is_file() else None,
        "artifacts": {"log": artifact(log)},
    }
    if completed.returncode != 0 or not output.is_file():
        result["status"] = "Umber-failure"
        return result
    compare = [str(args.parity_harness), "--compare-existing-dvi", str(expected_dvi),
               str(output), "--label", str(row["id"]), "--triage-dir", str(row_dir / "triage")]
    with comparison.open("wb") as out:
        compared = subprocess.run(compare, cwd=run, stdout=out, stderr=subprocess.STDOUT,
                                  check=False)
    comparison_text = comparison.read_text(errors="replace")
    if compared.returncode == 0:
        status = "DVI-exact"
    elif compared.returncode == 1 and re.search(r" DVI mismatch at byte \d+ on page ", comparison_text):
        status = "DVI-diverged"
    else:
        status = "comparison-error"
    result.update(comparison_command=compare, comparison_exit_status=compared.returncode,
                  comparison_log=artifact(comparison),
                  status=status)
    return result


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source-lock", "archives", "pdf-survey", "oracle", "oracle-build-record",
                 "reference-format", "format-receipt", "runtime-root", "runtime-lock",
                 "umber", "umber-format", "distribution", "parity-harness", "results"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=int, default=120)
    parser.add_argument("--max-rss-mib", type=int, default=1536)
    parser.add_argument("--distribution-ahash64", required=True)
    parser.add_argument("--verify-only", action="store_true")
    args = parser.parse_args()
    for name in ("source_lock", "archives", "pdf_survey", "oracle", "oracle_build_record",
                 "reference_format", "format_receipt", "runtime_root", "runtime_lock",
                 "umber", "umber_format", "distribution", "parity_harness", "results"):
        setattr(args, name, getattr(args, name).resolve())
    if not 1 <= args.timeout_seconds <= 1800 or not 1 <= args.max_rss_mib <= 6144:
        fail("guard limits are outside supported range")
    if not re.fullmatch(r"[0-9a-f]{16}", args.distribution_ahash64):
        fail("distribution aHash64 must be sixteen lowercase hexadecimal digits")
    return args


def main() -> int:
    args = parse_args()
    rows, pdf_metadata = verify_pdf_survey(args)
    for path in (args.umber, args.umber_format, args.parity_harness):
        if not path.is_file():
            fail(f"required cohort input is missing: {path}")
    if not args.distribution.is_dir():
        fail(f"Umber distribution is missing: {args.distribution}")
    manifest = args.distribution / "manifest.json"
    if not manifest.is_file() or json.loads(manifest.read_text()).get("schema") != 8:
        fail("Umber distribution must have a schema-8 manifest.json")
    if ahash64_file(manifest) != args.distribution_ahash64:
        fail("distribution manifest differs from --distribution-ahash64")
    root = json.loads(manifest.read_text())
    format_record = root.get("formats", {}).get("pdflatex")
    if (not isinstance(format_record, dict)
            or not re.fullmatch(r"[0-9a-f]{16}", str(format_record.get("ahash64", "")))
            or format_record.get("object") != f"ahash64-v1-{format_record['ahash64']}"
            or format_record.get("bytes") != args.umber_format.stat().st_size
            or ahash64_file(args.umber_format) != format_record["ahash64"]):
        fail("Umber format differs from authenticated distribution pdflatex record")
    run_identity = {
        "schema": 1, "source_lock": identity(args.source_lock),
        "pdf_survey_metadata": identity(args.pdf_survey / "metadata.json"),
        "pdf_survey_results": identity(args.pdf_survey / "results.jsonl"),
        "pdf_survey_summary": identity(args.pdf_survey / "summary.json"),
        "oracle": pdf_metadata["authority"], "umber": identity(args.umber),
        "umber_format": identity(args.umber_format), "distribution_manifest": identity(manifest),
        "distribution_ahash64": args.distribution_ahash64,
        "parity_harness": identity(args.parity_harness),
        "timeout_seconds": args.timeout_seconds, "max_rss_mib": args.max_rss_mib,
        "expansion_fuel": 500000000, "execution_steps": 10000000,
    }
    args.results.mkdir(parents=True, exist_ok=True)
    with (args.results / "run.lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            fail(f"another cohort run owns {args.results}")
        identity_path = args.results / "run-identity.json"
        if identity_path.is_file():
            if json.loads(identity_path.read_text()) != run_identity:
                fail("cohort run identity changed")
        elif args.verify_only:
            fail("no cohort run exists to verify")
        else:
            write_json(identity_path, run_identity)
        row_root = args.results / "rows"
        row_root.mkdir(exist_ok=True)
        reports = []
        missing_prefix = False
        for source, pdf in rows:
            row_id = str(source["id"])
            row_dir = row_root / row_id.replace("/", "_")
            receipt = row_dir / "result.json"
            if receipt.is_file():
                if missing_prefix:
                    fail(f"cohort has a missing earlier row before {row_id}")
                result = json.loads(receipt.read_text())
                check_record(result, source, pdf, args)
            elif args.verify_only:
                missing_prefix = True
                continue
            else:
                if row_dir.exists():
                    fail(f"cohort row receipt is missing from existing directory: {row_id}")
                row_dir.mkdir()
                result = {
                    "schema": 1, "id": row_id, "lock_order": source["lock_order"],
                    "source": source["source_identity"], "entrypoint": source["entrypoint"],
                    "jobname": source_jobname(str(source["entrypoint"])),
                    "pdf": pdf.get("pdf"), "pdf_pages": (pdf.get("pdf") or {}).get("pages"),
                }
                if pdf["terminal_status"]["classification"] != "PDF-success":
                    result["status"] = "PDF-ineligible"
                else:
                    reference = run_reference(args, source, row_dir)
                    result["reference"] = reference
                    if reference["status"] != "DVI-success":
                        result["status"] = "DVI-ineligible"
                    else:
                        umber = run_umber(args, source, row_dir, Path(str(reference["dvi"]["path"])))
                        result["umber"] = umber
                        result["status"] = umber["status"]
                write_json(receipt, result)
                check_record(result, source, pdf, args)
            reports.append(result)
            print(f"{row_id}: {result['status']}", flush=True)
            if result["status"] in ("Umber-failure", "DVI-diverged", "comparison-error"):
                break
        divergent = next((row["id"] for row in reports
                          if row["status"] in ("Umber-failure", "DVI-diverged")), None)
        comparison_error = next((row["id"] for row in reports
                                 if row["status"] == "comparison-error"), None)
        verdict = ("ERROR" if comparison_error else "DIVERGED" if divergent else
                   "COMPLETE" if len(reports) == len(rows) else "PARTIAL")
        write_json(args.results / "summary.json", {
            "schema": 1, "rows_recorded": len(reports), "sample_rows": pdf_metadata["scope"]["sample_rows"],
            "declared_compilers": pdf_metadata["scope"]["declared_compilers"],
            "declared_pdflatex_rows": len(rows), "verdict": verdict,
            "counts": {status: sum(row["status"] == status for row in reports)
                       for status in sorted({row["status"] for row in reports})},
            "first_divergence": divergent,
            "comparison_error": comparison_error,
        })
        if args.verify_only:
            print(f"verified {len(reports)} completed cohort rows without compiler launches")
        print(f"VERDICT: {verdict} ({len(reports)}/{len(rows)} declared-pdfLaTeX rows)")
        return {"COMPLETE": 0, "DIVERGED": 1, "PARTIAL": 2, "ERROR": 3}[verdict]


if __name__ == "__main__":
    sys.exit(main())
