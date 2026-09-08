#!/usr/bin/env python3
"""Run paired measurements against already-built executables."""

from __future__ import annotations

import argparse
import datetime as datetime_module
import sys
import tempfile
from pathlib import Path
from typing import Any

from paired_measure import (
    RUNNER, SCHEMA, RunnerError,
    compare_sample, require,
    choose_time_command, host_identity, load_manifest, make_pair,
    parse_cpu_list, run_sample, summarize_workload, verify_identity,
    workload_manifest_digest, write_record,
)


def run_measurement(arguments: argparse.Namespace) -> dict[str, Any]:
    require(arguments.pairs > 0, "--pairs must be positive")
    require(arguments.warmups >= 0, "--warmups cannot be negative")
    require(arguments.timeout_seconds > 0, "--timeout-seconds must be positive")
    require(arguments.max_output_bytes > 0, "--max-output-bytes must be positive")
    manifest_path = arguments.manifest.resolve()
    manifest, manifest_hash = load_manifest(manifest_path, arguments.lane)
    baseline = verify_identity(arguments.baseline_worktree, arguments.baseline_revision, arguments.baseline_binary)
    candidate = verify_identity(arguments.candidate_worktree, arguments.candidate_revision, arguments.candidate_binary)
    time_command = choose_time_command(arguments.time_command)
    cpus = parse_cpu_list(arguments.cpu_list)
    global_environment = manifest.get("environment", {})
    workload_digest = workload_manifest_digest(manifest["workloads"], global_environment)
    output_path = arguments.output.resolve()
    require(not output_path.exists(), f"output already exists: {output_path}")
    output_path.parent.mkdir(parents=True, exist_ok=True)
    run_root = (arguments.run_root or output_path.parent / f"{output_path.stem}.runs").resolve()
    run_root.mkdir(parents=True, exist_ok=True)
    metadata = {
        "record": "metadata",
        "schema": SCHEMA,
        "runner": RUNNER,
        "lane": arguments.lane,
        "profile": manifest["profile"],
        "features": manifest["features"],
        "instrumented": manifest["instrumented"],
        "build": manifest.get("build", {}),
        "toolchain": manifest.get("toolchain", {}),
        "manifest": {"path": str(manifest_path), "sha256": manifest_hash},
        "workload_manifest_sha256": workload_digest,
        "baseline": baseline,
        "candidate": candidate,
        "host": host_identity(),
        "configuration": {
            "pairs": arguments.pairs,
            "warmups": arguments.warmups,
            "timeout_seconds": arguments.timeout_seconds,
            "max_output_bytes": arguments.max_output_bytes,
            "time_command": str(time_command) if time_command is not None else None,
            "cpu_list": sorted(cpus) if cpus is not None else None,
            "run_root": str(run_root),
            "environment_overrides": global_environment,
        },
        "started_at_utc": datetime_module.datetime.now(datetime_module.timezone.utc).isoformat(),
    }
    workload_summaries: list[dict[str, Any]] = []
    failure: str | None = None
    with output_path.open("x", encoding="utf-8") as receipt:
        write_record(receipt, metadata)
        try:
            for workload in manifest["workloads"]:
                samples: list[dict[str, Any]] = []
                pair_records: list[dict[str, Any]] = []
                reference: dict[str, Any] | None = None
                for pair_index in range(arguments.pairs):
                    order = "AB" if pair_index % 2 == 0 else "BA"
                    sides = ("baseline", "candidate") if order == "AB" else ("candidate", "baseline")
                    measured_by_side: dict[str, dict[str, Any]] = {}
                    for side in sides:
                        side_identity = baseline if side == "baseline" else candidate
                        binary = Path(side_identity["binary"]["path"])
                        environment = dict(global_environment)
                        environment.update(workload.get("environment", {}))
                        for warmup_index in range(arguments.warmups):
                            warmup_dir = Path(tempfile.mkdtemp(prefix=f"{workload['name']}-p{pair_index}-{side}-w{warmup_index}-", dir=run_root))
                            sample = run_sample(
                                side=side,
                                binary=binary,
                                workload=workload,
                                sample_dir=warmup_dir,
                                environment=environment,
                                timeout_seconds=arguments.timeout_seconds,
                                time_command=time_command,
                                cpus=cpus,
                                max_output_bytes=arguments.max_output_bytes,
                            )
                            sample.update({"record": "sample", "schema": SCHEMA, "lane": arguments.lane, "sample_id": f"{workload['name']}-p{pair_index}-{side}-warmup-{warmup_index}", "pair_index": pair_index, "order": order, "role": "warmup"})
                            if sample["validation_errors"]:
                                write_record(receipt, sample)
                                raise RunnerError(
                                    f"{workload['name']} {side} warm-up failed: {'; '.join(sample['validation_errors'])}"
                                )
                            samples.append(sample)
                            write_record(receipt, sample)
                        measured_dir = Path(tempfile.mkdtemp(prefix=f"{workload['name']}-p{pair_index}-{side}-m-", dir=run_root))
                        sample = run_sample(
                            side=side,
                            binary=binary,
                            workload=workload,
                            sample_dir=measured_dir,
                            environment=environment,
                            timeout_seconds=arguments.timeout_seconds,
                            time_command=time_command,
                            cpus=cpus,
                            max_output_bytes=arguments.max_output_bytes,
                        )
                        sample.update({"record": "sample", "schema": SCHEMA, "lane": arguments.lane, "sample_id": f"{workload['name']}-p{pair_index}-{side}-measure", "pair_index": pair_index, "order": order, "role": "measure"})
                        if sample["validation_errors"]:
                            write_record(receipt, sample)
                            raise RunnerError(
                                f"{workload['name']} {side} measurement failed: {'; '.join(sample['validation_errors'])}"
                            )
                        if side == "baseline" and reference is None:
                            reference = sample
                        elif reference is not None:
                            errors = compare_sample(sample, reference, workload)
                            if errors:
                                write_record(receipt, sample)
                                raise RunnerError(
                                    f"{workload['name']} {side} measurement failed validation: {'; '.join(errors)}"
                                )
                        samples.append(sample)
                        measured_by_side[side] = sample
                        write_record(receipt, sample)
                    pair = make_pair(pair_index, order, measured_by_side["baseline"], measured_by_side["candidate"])
                    pair.update({"schema": SCHEMA, "lane": arguments.lane})
                    pair_records.append(pair)
                    write_record(receipt, pair)
                workload_summary = summarize_workload(workload["name"], samples, pair_records)
                workload_summaries.append(workload_summary)
            for label, identity, worktree, revision, binary in (
                ("baseline", baseline, arguments.baseline_worktree, arguments.baseline_revision, arguments.baseline_binary),
                ("candidate", candidate, arguments.candidate_worktree, arguments.candidate_revision, arguments.candidate_binary),
            ):
                final_identity = verify_identity(worktree, revision, binary)
                require(
                    final_identity["binary"]["sha256"] == identity["binary"]["sha256"],
                    f"{label} binary changed during measurement",
                )
            summary = {
                "record": "summary",
                "schema": SCHEMA,
                "status": "PASS",
                "lane": arguments.lane,
                "workloads": workload_summaries,
            }
        except RunnerError as error:
            failure = str(error)
            summary = {
                "record": "summary",
                "schema": SCHEMA,
                "status": "FAIL",
                "lane": arguments.lane,
                "workloads": workload_summaries,
                "error": failure,
            }
        write_record(receipt, summary)
    if failure is not None:
        raise RunnerError(failure)
    return summary


def argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--lane", choices=("release-unobserved", "profiling-structural"), required=True)
    parser.add_argument("--baseline-worktree", type=Path, required=True)
    parser.add_argument("--baseline-revision", required=True)
    parser.add_argument("--baseline-binary", type=Path, required=True)
    parser.add_argument("--candidate-worktree", type=Path, required=True)
    parser.add_argument("--candidate-revision", required=True)
    parser.add_argument("--candidate-binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--run-root", type=Path)
    parser.add_argument("--pairs", type=int, default=5)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--timeout-seconds", type=float, default=600.0)
    parser.add_argument("--max-output-bytes", type=int, default=64 * 1024 * 1024)
    parser.add_argument("--cpu-list")
    parser.add_argument("--time-command")
    return parser


def main(argv: list[str] | None = None) -> int:
    arguments = argument_parser().parse_args(argv)
    try:
        summary = run_measurement(arguments)
    except RunnerError as error:
        print(f"paired-measure: FAIL: {error}", file=sys.stderr)
        return 2
    print(
        f"paired-measure: PASS lane={summary['lane']} workloads={len(summary['workloads'])} "
        f"pairs={sum(item['pair_count'] for item in summary['workloads'])}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
