#!/usr/bin/env python3
"""Run the locked arXiv sources with each archive's declared TeX Live year."""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import fcntl
import json
import os
import re
import resource
import subprocess
import sys
from pathlib import Path

from arxiv_corpus import (archive_members, declared_texlive, materialize, sha256_file,
                          source_identity, source_jobname, verify_view)
from arxiv_texlive_inputs import audit_umber_inputs
from texlive import ahash64_file

ROOT = Path(__file__).resolve().parent.parent
SUPPORTED_ENGINES = ("latex", "pdflatex")
DIVERGENCES = ("Umber-failure", "DVI-diverged")


def fail(message: str) -> None:
    raise SystemExit(message)


def identity(path: Path) -> dict[str, object]:
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256_file(path)}


def write_json(path: Path, value: object) -> None:
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n")
    os.replace(temporary, path)


def artifact(path: Path) -> dict[str, object] | None:
    return identity(path) if path.is_file() else None


def check_artifact(record: object, results: Path) -> None:
    if record is None:
        return
    if not isinstance(record, dict) or not isinstance(record.get("path"), str):
        fail("invalid corpus artifact receipt")
    path = Path(record["path"])
    if not path.is_relative_to(results) or not path.is_file() or identity(path) != record:
        fail(f"corpus artifact changed: {path}")


def dvi_pages(path: Path) -> int:
    data = path.read_bytes()
    last = len(data) - 1
    while last >= 0 and data[last] == 223:
        last -= 1
    if last < 5 or data[last - 5] != 249:
        raise ValueError("missing DVI post_post trailer")
    post = int.from_bytes(data[last - 4:last], "big")
    if post + 29 > len(data) or data[post] != 248:
        raise ValueError("invalid DVI postamble")
    return int.from_bytes(data[post + 27:post + 29], "big")


def check_recorder(path: Path, run: Path, runtime: Path, fmt: Path, fontmaps: Path) -> None:
    """Reject every recorded input outside the source job and selected release."""
    if not path.is_file():
        fail(f"successful reference DVI has no recorder trace: {path}")
    roots = (run.resolve(), runtime.resolve(), fontmaps.resolve())
    for line in path.read_text(errors="replace").splitlines():
        if not line.startswith("INPUT "):
            continue
        source = Path(line[6:])
        resolved = (source if source.is_absolute() else run / source).resolve()
        if "latex-dev" in resolved.parts:
            fail(f"reference selected latex-dev input: {resolved}")
        if resolved not in (fmt.resolve(), (fmt.parent / "generated-config/language.dat").resolve()) and not any(resolved.is_relative_to(root) for root in roots):
            fail(f"reference recorder escaped selected source/runtime: {resolved}")


def preserve_incomplete(path: Path, parent: Path, stem: str) -> None:
    """Keep a failed attempt while freeing its stable path for a fresh run."""
    if not path.exists():
        return
    parent.mkdir(parents=True, exist_ok=True)
    for number in range(1, 10_000):
        destination = parent / f"{stem}-{number:04d}"
        if not destination.exists():
            os.replace(path, destination)
            return
    fail(f"too many retained incomplete attempts for {path}")


def preserve_incomplete_umber(row_dir: Path) -> None:
    names = ("umber", "umber.log", "umber.inputs", "comparison.log", "triage")
    present = [name for name in names if (row_dir / name).exists()]
    if not present:
        return
    attempts = row_dir / "incomplete-umber"
    attempts.mkdir(exist_ok=True)
    for number in range(1, 10_000):
        destination = attempts / f"{number:04d}"
        if not destination.exists():
            destination.mkdir()
            for name in present:
                os.replace(row_dir / name, destination / name)
            return
    fail(f"too many retained incomplete Umber attempts for {row_dir}")


def read_sources(lock: Path, archives: Path, expected_rows: int) -> list[dict]:
    with lock.open(newline="") as source:
        reader = csv.DictReader(source, delimiter="\t")
        if reader.fieldnames != ["id", "source_sha256", "source_bytes", "first_submitted",
                                  "shuffle_sha256", "entrypoint"]:
            fail("unexpected arXiv source lock schema")
        raw_rows = list(reader)
    if len(raw_rows) != expected_rows:
        fail(f"source lock has {len(raw_rows)} rows; expected {expected_rows}")
    rows = []
    seen = set()
    for order, row in enumerate(raw_rows, 1):
        row_id = row["id"]
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", row_id) or row_id in seen:
            fail(f"invalid or duplicate source id: {row_id!r}")
        seen.add(row_id)
        archive = archives / f"{row_id}.src"
        if not archive.is_file() or archive.stat().st_size != int(row["source_bytes"]):
            fail(f"missing or changed source archive: {archive}")
        if sha256_file(archive) != row["source_sha256"]:
            fail(f"source archive differs from lock: {archive}")
        compiler, year, declaration = declared_texlive(archive)
        source = source_identity(archive, row["entrypoint"])
        if row["entrypoint"] not in {str(member["path"]) for member in archive_members(archive)}:
            fail(f"locked entrypoint absent from archive: {row_id}")
        rows.append({"id": row_id, "lock_order": order, "archive": archive,
                     "entrypoint": row["entrypoint"], "jobname": source_jobname(row["entrypoint"]),
                     "source": source, "compiler": compiler, "year": year,
                     "declaration": declaration})
    return rows


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
        if (not admission.is_file() or not admission.resolve().is_relative_to(format_root)
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


def reference_environment(runtime: Path, run: Path, fmt: Path, epoch: int,
                          view: Path, fontmaps: Path) -> dict[str, str]:
    from texlive_formats import stable_paths

    selected = view / "texmf-dist"
    base, kernel, ini = stable_paths(selected)
    config = fmt.parent / "generated-config"
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("TEX", "TFMF")) and not key.endswith("FONTS")
           and key != "OSFONTDIR"}
    for key, name in {"TEXMFVAR": "texmf-var", "TEXMFCONFIG": "texmf-config",
                      "TEXMFHOME": "texmf-home", "TEXMFSYSVAR": "texmf-sysvar",
                      "TEXMFSYSCONFIG": "texmf-sysconfig", "TEXMFLOCAL": "texmf-local",
                      "TEXMFCACHE": "texmf-cache", "VARTEXFONTS": "texmf-var/fonts"}.items():
        path = run / name
        path.mkdir(parents=True, exist_ok=True)
        env[key] = str(path)
    env.update({"TEXMFCNF": str(runtime / "web2c"), "TEXMFROOT": str(view),
                "TEXMFDIST": str(selected), "TEXMFDBS": f"!!{view}", "TEXFORMATS": str(fmt.parent),
                "TEXINPUTS": ":".join((str(run), str(config), f"!!{base}", f"!!{kernel}", f"!!{ini}",
                                       f"!!{selected}/tex/latex//", f"!!{selected}/tex/generic//",
                                       f"!!{selected}/tex/plain//", f"!!{selected}/tex//")),
                "TFMFONTS": f"{run}:!!{selected}/fonts/tfm//",
                "TEXFONTMAPS": f"{fontmaps}/fonts/map//:!!{selected}/fonts/map//",
                "SOURCE_DATE_EPOCH": str(epoch),
                "FORCE_SOURCE_DATE": "1"})
    return env


def memory_limit(mib: int):
    def limit() -> None:
        resource.setrlimit(resource.RLIMIT_AS, (mib * 1024 * 1024, mib * 1024 * 1024))
    return limit


def run_reference(args: argparse.Namespace, row: dict, row_dir: Path, proof: dict) -> dict:
    run = row_dir / "reference"
    materialize(row["archive"], run)
    verify_view(row["archive"], run)
    output = run / f"{row['jobname']}.dvi"
    log = run / f"{row['jobname']}.log"
    recorder = run / f"{row['jobname']}.fls"
    stdout, stderr, timing = (row_dir / name for name in
                              ("reference.stdout", "reference.stderr", "reference.time"))
    binary = proof["reference_binary"]["path"]
    fmt = proof["reference_format"]["path"]
    command = ["/usr/bin/time", "-v", "-o", str(timing), "timeout", "-k", "2s",
               f"{args.timeout_seconds}s", binary, f"-progname={row['compiler']}",
               f"--fmt={fmt}", "--output-format=dvi", "-recorder", "--interaction=nonstopmode",
               "--halt-on-error", row["entrypoint"]]
    with stdout.open("wb") as out, stderr.open("wb") as err:
        completed = subprocess.run(command, cwd=run,
                                   env=reference_environment(Path(proof["runtime_root"]), run, Path(fmt),
                                                             proof["source_date_epoch"], Path(proof["reference_view"]),
                                                             Path(proof["generated_fontmaps"])),
                                   stdout=out, stderr=err, check=False,
                                   preexec_fn=memory_limit(args.max_rss_mib))
    pages = None
    if output.is_file():
        try:
            pages = dvi_pages(output)
        except ValueError:
            pass
    success = completed.returncode == 0 and pages is not None and recorder.is_file()
    if recorder.is_file():
        check_recorder(recorder, run, Path(proof["runtime_root"]), Path(fmt), Path(proof["generated_fontmaps"]))
    return {"command": command, "working_directory": str(run),
            "exit_status": completed.returncode, "status": "DVI-success" if success else "DVI-failure",
            "pages": pages, "dvi": artifact(output),
            "artifacts": {"log": artifact(log), "recorder": artifact(recorder), "stdout": artifact(stdout),
                          "stderr": artifact(stderr), "time": artifact(timing)}}


def run_umber(args: argparse.Namespace, row: dict, row_dir: Path, proof: dict, expected: Path) -> dict:
    run = row_dir / "umber"
    materialize(row["archive"], run)
    verify_view(row["archive"], run)
    output = run / f"{row['jobname']}.dvi"
    log, comparison = row_dir / "umber.log", row_dir / "comparison.log"
    admission = row_dir / "umber.inputs"
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("TEX", "TFMF")) and not key.endswith("FONTS")
           and key != "OSFONTDIR" and not key.startswith("UMBER_")}
    env.update({"SOURCE_DATE_EPOCH": str(proof["source_date_epoch"]), "FORCE_SOURCE_DATE": "1",
                "TEXINPUTS": str(run), "TEXFONTS": str(run)})
    command = [sys.executable, str(ROOT / "scripts/run-umber-guarded.py"),
               "--timeout-seconds", str(args.timeout_seconds), "--max-rss-mib",
               str(args.max_rss_mib), "--term-grace-seconds", "2", "--", str(args.umber),
               "run", f"--{row['compiler']}", "--distribution", proof["umber_distribution"],
               "--distribution-ahash64", proof["distribution_ahash64"],
               "--format", proof["umber_format"]["path"], "--offline",
               "--expansion-fuel", "500000000", "--execution-steps", "10000000",
               "--dvi", str(output), "--input-records-out", str(admission), row["entrypoint"]]
    with log.open("wb") as out:
        completed = subprocess.run(command, cwd=run, env=env, stdout=out,
                                   stderr=subprocess.STDOUT, check=False)
    result = {"command": command, "working_directory": str(run),
              "exit_status": completed.returncode, "dvi": artifact(output),
              "artifacts": {"log": artifact(log), "inputs": artifact(admission)}}
    if completed.returncode or not output.is_file():
        result["status"] = "Umber-failure"
        return result
    try:
        result["pages"] = dvi_pages(output)
    except ValueError:
        result["status"] = "Umber-failure"
        return result
    if not admission.is_file():
        fail(f"successful Umber DVI has no input admission receipt: {row['id']}")
    result["input_audit"] = audit_umber_inputs(row, row_dir, proof, admission)
    compare = [str(args.parity_harness), "--compare-existing-dvi", str(expected),
               str(output), "--label", row["id"], "--triage-dir", str(row_dir / "triage")]
    with comparison.open("wb") as out:
        compared = subprocess.run(compare, cwd=run, stdout=out, stderr=subprocess.STDOUT,
                                  check=False)
    output_text = comparison.read_text(errors="replace")
    if compared.returncode == 0:
        status = "DVI-exact"
    elif compared.returncode == 1 and re.search(r" DVI mismatch at byte \d+ on page ", output_text):
        status = "DVI-diverged"
    else:
        status = "comparison-error"
    result.update(status=status, comparison_command=compare,
                  comparison_exit_status=compared.returncode,
                  comparison_log=artifact(comparison))
    return result


def check_result(result: dict, row: dict, proof: dict | None, results: Path,
                 args: argparse.Namespace) -> None:
    expected = {key: row[key] for key in ("id", "lock_order", "entrypoint", "jobname",
                                          "source", "compiler", "year", "declaration")}
    if any(result.get(key) != value for key, value in expected.items()):
        fail(f"corpus row source/year declaration changed: {row['id']}")
    if result.get("authority") != proof:
        fail(f"corpus row authority changed: {row['id']}")
    if proof is None:
        if result.get("status") != "unsupported-xelatex" or "reference" in result:
            fail(f"invalid unsupported compiler row: {row['id']}")
        return
    reference = result.get("reference")
    if not isinstance(reference, dict):
        fail(f"reference qualification missing: {row['id']}")
    row_dir = results / "rows" / row["id"]
    expected_command = ["/usr/bin/time", "-v", "-o", str(row_dir / "reference.time"),
                        "timeout", "-k", "2s", f"{args.timeout_seconds}s",
                        proof["reference_binary"]["path"], f"-progname={row['compiler']}",
                        f"--fmt={proof['reference_format']['path']}", "--output-format=dvi", "-recorder",
                        "--interaction=nonstopmode", "--halt-on-error", row["entrypoint"]]
    if reference.get("command") != expected_command or reference.get("working_directory") != str(row_dir / "reference"):
        fail(f"reference command changed: {row['id']}")
    for item in (reference.get("dvi"), *(reference.get("artifacts") or {}).values()):
        check_artifact(item, results)
    if reference.get("status") == "DVI-failure":
        if (result.get("status") != "DVI-ineligible" or "umber" in result
                or reference.get("exit_status") == 0 and reference.get("dvi") is not None
                and reference.get("pages") is not None
                and (reference.get("artifacts") or {}).get("recorder") is not None):
            fail(f"invalid reference failure row: {row['id']}")
        return
    dvi = reference.get("dvi")
    if (reference.get("status") != "DVI-success" or reference.get("exit_status") != 0
            or dvi is None or (reference.get("artifacts") or {}).get("recorder") is None
            or reference.get("pages") != dvi_pages(Path(dvi["path"]))):
        fail(f"invalid successful reference DVI: {row['id']}")
    check_recorder(row_dir / "reference" / f"{row['jobname']}.fls",
                   row_dir / "reference", Path(proof["runtime_root"]),
                   Path(proof["reference_format"]["path"]), Path(proof["generated_fontmaps"]))
    umber = result.get("umber")
    if umber is None:
        if result.get("status") != "DVI-qualified":
            fail(f"invalid pending parity row: {row['id']}")
        return
    if not isinstance(umber, dict) or result.get("status") != umber.get("status"):
        fail(f"invalid Umber result: {row['id']}")
    expected_umber = [sys.executable, str(ROOT / "scripts/run-umber-guarded.py"),
                      "--timeout-seconds", str(args.timeout_seconds), "--max-rss-mib",
                      str(args.max_rss_mib), "--term-grace-seconds", "2", "--", str(args.umber),
                      "run", f"--{row['compiler']}", "--distribution", proof["umber_distribution"],
                      "--distribution-ahash64", proof["distribution_ahash64"],
                      "--format", proof["umber_format"]["path"], "--offline",
                      "--expansion-fuel", "500000000", "--execution-steps", "10000000",
                      "--dvi", str(row_dir / "umber" / f"{row['jobname']}.dvi"),
                      "--input-records-out", str(row_dir / "umber.inputs"), row["entrypoint"]]
    if umber.get("command") != expected_umber or umber.get("working_directory") != str(row_dir / "umber"):
        fail(f"Umber command changed: {row['id']}")
    for item in (umber.get("dvi"), *(umber.get("artifacts") or {}).values(),
                 umber.get("comparison_log")):
        check_artifact(item, results)
    if umber.get("status") == "Umber-failure":
        if umber.get("exit_status") == 0 and umber.get("dvi") is not None and umber.get("pages") is not None:
            fail(f"invalid Umber failure row: {row['id']}")
        return
    if umber.get("exit_status") != 0 or umber.get("dvi") is None:
        fail(f"missing successful Umber DVI: {row['id']}")
    if umber.get("pages") != dvi_pages(Path(umber["dvi"]["path"])):
        fail(f"Umber DVI page count changed: {row['id']}")
    admission = (umber.get("artifacts") or {}).get("inputs")
    if admission is None or umber.get("input_audit") != audit_umber_inputs(
            row, row_dir, proof, Path(admission["path"])):
        fail(f"Umber input audit changed: {row['id']}")
    comparison_command = [str(args.parity_harness), "--compare-existing-dvi", dvi["path"],
                          umber["dvi"]["path"], "--label", row["id"],
                          "--triage-dir", str(row_dir / "triage")]
    if umber.get("comparison_command") != comparison_command or umber.get("comparison_log") is None:
        fail(f"DVI comparator command changed: {row['id']}")
    comparison_text = Path(umber["comparison_log"]["path"]).read_text(errors="replace")
    expected_status = ("DVI-exact" if umber.get("comparison_exit_status") == 0 else
                       "DVI-diverged" if umber.get("comparison_exit_status") == 1 and
                       re.search(r" DVI mismatch at byte \d+ on page ", comparison_text) else
                       "comparison-error")
    if result["status"] != expected_status:
        fail(f"DVI comparison status changed: {row['id']}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source-lock", "archives", "preparation", "umber", "parity-harness", "results"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--expected-rows", type=int, default=100)
    parser.add_argument("--timeout-seconds", type=int, default=120)
    parser.add_argument("--max-rss-mib", type=int, default=1536)
    parser.add_argument("--verify-only", action="store_true")
    parser.add_argument("--qualify-only", action="store_true")
    args = parser.parse_args()
    for name in ("source_lock", "archives", "preparation", "umber", "parity_harness", "results"):
        setattr(args, name, getattr(args, name).resolve())
    if args.expected_rows < 1 or not 1 <= args.timeout_seconds <= 1800 or not 1 <= args.max_rss_mib <= 6144:
        fail("corpus row count or guard limits are outside supported range")
    return args


def main() -> int:
    args = parse_args()
    rows = read_sources(args.source_lock, args.archives, args.expected_rows)
    for path in (args.umber, args.parity_harness):
        if not path.is_file() or not os.access(path, os.X_OK):
            fail(f"required corpus binary is missing or not executable: {path}")
    _, authorities = authority(args.preparation, rows)
    run_identity = {"schema": 1, "source_lock": identity(args.source_lock),
                    "preparation": identity(args.preparation), "authorities": authorities,
                    "umber": identity(args.umber), "parity_harness": identity(args.parity_harness),
                    "timeout_seconds": args.timeout_seconds, "max_rss_mib": args.max_rss_mib,
                    "expansion_fuel": 500000000, "execution_steps": 10000000}
    args.results.mkdir(parents=True, exist_ok=True)
    with (args.results / "run.lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            fail(f"another corpus run owns {args.results}")
        identity_path = args.results / "run-identity.json"
        if identity_path.is_file():
            if json.loads(identity_path.read_text()) != run_identity:
                fail("corpus run authority changed")
        elif args.verify_only:
            fail("no corpus run exists to verify")
        else:
            write_json(identity_path, run_identity)
        row_root = args.results / "rows"
        row_root.mkdir(exist_ok=True)
        existing = [(row_root / row["id"] / "result.json").is_file() for row in rows]
        if any(later and not earlier for earlier, later in zip(existing, existing[1:])):
            fail("corpus has a missing earlier row receipt before a later row")
        reports = []
        for row in rows:
            row_dir = row_root / row["id"]
            receipt = row_dir / "result.json"
            proof = authorities.get(f"{row['year']}/{row['compiler']}")
            if receipt.is_file():
                result = json.loads(receipt.read_text())
                check_result(result, row, proof, args.results, args)
            elif args.verify_only:
                continue
            else:
                if row_dir.exists():
                    preserve_incomplete(row_dir, row_root / "incomplete-reference", row["id"])
                row_dir.mkdir()
                result = {key: row[key] for key in ("id", "lock_order", "entrypoint", "jobname",
                                                    "source", "compiler", "year", "declaration")}
                result["schema"] = 1
                result["authority"] = proof
                if proof is None:
                    result["status"] = "unsupported-xelatex"
                else:
                    reference = run_reference(args, row, row_dir, proof)
                    result["reference"] = reference
                    result["status"] = ("DVI-qualified" if reference["status"] == "DVI-success"
                                        else "DVI-ineligible")
                write_json(receipt, result)
                check_result(result, row, proof, args.results, args)
            reports.append(result)
            print(f"reference {row['id']}: {result['status']}", flush=True)
        if not args.verify_only and not args.qualify_only:
            for row, result in zip(rows, reports):
                if result["status"] in DIVERGENCES or result["status"] == "comparison-error":
                    break
                if result["status"] != "DVI-qualified":
                    continue
                row_dir = row_root / row["id"]
                proof = authorities[f"{row['year']}/{row['compiler']}"]
                preserve_incomplete_umber(row_dir)
                result["umber"] = run_umber(args, row, row_dir, proof,
                                             Path(result["reference"]["dvi"]["path"]))
                result["status"] = result["umber"]["status"]
                write_json(row_dir / "result.json", result)
                check_result(result, row, proof, args.results, args)
                print(f"parity {row['id']}: {result['status']}", flush=True)
                if result["status"] in (*DIVERGENCES, "comparison-error"):
                    break
        divergent = next((result["id"] for result in reports if result["status"] in DIVERGENCES), None)
        comparison_error = next((result["id"] for result in reports
                                 if result["status"] == "comparison-error"), None)
        complete = len(reports) == len(rows) and all(result["status"] != "DVI-qualified" for result in reports)
        verdict = ("ERROR" if comparison_error else "DIVERGED" if divergent else
                   "COMPLETE" if complete else "PARTIAL")
        summary = {"schema": 1, "sample_rows": len(rows), "reference_rows_recorded": len(reports),
                   "dvi_eligible_rows": sum(result["reference"]["status"] == "DVI-success"
                                            for result in reports if "reference" in result),
                   "verdict": verdict, "counts": {status: sum(result["status"] == status for result in reports)
                                                  for status in sorted({result["status"] for result in reports})},
                   "first_divergence": divergent, "comparison_error": comparison_error}
        write_json(args.results / "summary.json", summary)
        print(f"VERDICT: {verdict} ({len(reports)}/{len(rows)} reference rows)")
        return {"COMPLETE": 0, "DIVERGED": 1, "PARTIAL": 2, "ERROR": 3}[verdict]


if __name__ == "__main__":
    sys.exit(main())
