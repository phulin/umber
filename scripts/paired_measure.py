"""Support library for the paired prebuilt-executable measurement runner."""

from __future__ import annotations

import hashlib
import json
import os
import platform
import re
import signal
import statistics
import subprocess
import time
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any, Iterable, Mapping

from paired_metrics import compare_inner_shapes, extract_inner_metrics, pair_inner_metrics, summarize_inner_metrics

try:
    import resource
except ImportError:  # pragma: no cover - Windows does not expose RUSAGE_CHILDREN.
    resource = None

SCHEMA = 1
RUNNER = "paired-measure-v1"
REVISION_RE = re.compile(r"^[0-9a-f]{40}$")
TIME_FORMAT = "%U\\t%S\\t%M"
MAX_SEMANTIC_PREVIEW_BYTES = 16 * 1024
METRICS = ("wall_ns", "user_cpu_ns", "system_cpu_ns", "max_rss_kib")

class RunnerError(RuntimeError):
    """A configuration, identity, process, or validation failure."""

def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")

def digest_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()

def digest_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def require(condition: bool, message: str) -> None:
    if not condition:
        raise RunnerError(message)

def load_manifest(path: Path, lane: str) -> tuple[dict[str, Any], str]:
    path = path.resolve()
    require(path.is_file(), f"manifest does not exist: {path}")
    raw = path.read_bytes()
    try:
        manifest = json.loads(raw)
    except json.JSONDecodeError as error:
        raise RunnerError(f"manifest is not valid JSON: {path}: {error}") from error
    require(isinstance(manifest, dict), "manifest root must be an object")
    require(manifest.get("schema") == SCHEMA, f"manifest schema must be {SCHEMA}")
    require(manifest.get("lane") == lane, "--lane must match manifest lane")
    profile = manifest.get("profile")
    features = manifest.get("features")
    instrumented = manifest.get("instrumented")
    require(isinstance(profile, str) and profile, "manifest profile must be a string")
    require(
        isinstance(features, list) and all(isinstance(item, str) for item in features),
        "manifest features must be a list of strings",
    )
    require(isinstance(instrumented, bool), "manifest instrumented must be boolean")
    for identity_name in ("build", "toolchain"):
        identity = manifest.get(identity_name, {})
        require(isinstance(identity, dict), f"manifest {identity_name} must be an object")
    if lane == "release-unobserved":
        require(profile == "release", "release-unobserved requires profile=release")
        require(not instrumented, "release-unobserved requires instrumented=false")
        require(
            not any("profil" in feature.lower() for feature in features),
            "release-unobserved cannot name a profiling feature",
        )
    elif lane == "profiling-structural":
        require(instrumented, "profiling-structural requires instrumented=true")
        require(
            any("profil" in feature.lower() for feature in features),
            "profiling-structural must name its profiling feature",
        )
    else:
        raise RunnerError(f"unsupported evidence lane: {lane}")

    workloads = manifest.get("workloads")
    require(isinstance(workloads, list) and workloads, "manifest workloads must be non-empty")
    normalized: list[dict[str, Any]] = []
    names: set[str] = set()
    for workload in workloads:
        normalized.append(normalize_workload(workload, names))
    manifest = dict(manifest)
    manifest["workloads"] = normalized
    environment = manifest.get("environment", {})
    require(
        isinstance(environment, dict)
        and all(isinstance(key, str) and isinstance(value, str) for key, value in environment.items()),
        "manifest environment must map strings to strings",
    )
    return manifest, digest_bytes(raw)

def normalize_file(path_value: Any) -> dict[str, Any]:
    if isinstance(path_value, str):
        path = Path(path_value)
        expected = None
    elif isinstance(path_value, dict):
        raw_path = path_value.get("path")
        require(isinstance(raw_path, str), "workload file path must be a string")
        path = Path(raw_path)
        expected = path_value.get("sha256")
        require(expected is None or isinstance(expected, str), "file sha256 must be a string")
    else:
        raise RunnerError("workload files must contain paths or objects")
    require(path.is_absolute(), f"workload file path must be absolute: {path}")
    path = path.resolve()
    require(path.is_file(), f"workload file does not exist: {path}")
    actual = digest_file(path)
    if expected is not None:
        require(actual == expected, f"workload file hash mismatch: {path}")
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": actual}

def normalize_workload(workload: Any, names: set[str]) -> dict[str, Any]:
    require(isinstance(workload, dict), "each workload must be an object")
    name = workload.get("name")
    args = workload.get("args", [])
    require(isinstance(name, str) and name, "workload name must be non-empty")
    require(name not in names, f"duplicate workload name: {name}")
    names.add(name)
    require(isinstance(args, list) and all(isinstance(item, str) for item in args), "workload args must be strings")
    files = workload.get("files", [])
    require(isinstance(files, list), f"workload files must be a list: {name}")
    outputs = workload.get("outputs", [])
    require(isinstance(outputs, list), f"workload outputs must be a list: {name}")
    normalized_outputs: list[dict[str, Any]] = []
    for output in outputs:
        require(isinstance(output, dict), f"workload output must be an object: {name}")
        raw_path = output.get("path")
        require(isinstance(raw_path, str) and raw_path, f"output path must be non-empty: {name}")
        relative = Path(raw_path)
        require(not relative.is_absolute(), f"output path must be relative to its sample directory: {raw_path}")
        require(".." not in relative.parts, f"output path escapes its sample directory: {raw_path}")
        expected = output.get("sha256")
        require(expected is None or isinstance(expected, str), "output sha256 must be a string")
        normalize = output.get("normalize", "raw")
        require(normalize == "raw", "only raw output validation is supported")
        normalized_outputs.append(
            {"path": str(relative), "required": output.get("required", True), "sha256": expected}
        )
        require(isinstance(output.get("required", True), bool), "output required must be boolean")
    semantic = workload.get("semantic")
    if semantic is not None:
        require(isinstance(semantic, dict), f"semantic configuration must be an object: {name}")
        stream = semantic.get("stream", "stdout")
        fmt = semantic.get("format", "json")
        ignored = semantic.get("ignore_keys", [])
        require(stream in {"stdout", "stderr"}, f"semantic stream is invalid: {name}")
        require(fmt in {"json", "jsonl"}, f"semantic format is invalid: {name}")
        require(
            isinstance(ignored, list) and all(isinstance(item, str) for item in ignored),
            f"semantic ignore_keys must be strings: {name}",
        )
    inner_metrics = workload.get("inner_metrics", [])
    require(isinstance(inner_metrics, list), f"inner_metrics must be a list: {name}")
    normalized_metrics: list[dict[str, Any]] = []
    metric_names: set[str] = set()
    for metric in inner_metrics:
        require(isinstance(metric, dict), f"inner metric must be an object: {name}")
        metric_name = metric.get("name")
        path = metric.get("path")
        key = metric.get("key", [])
        unit = metric.get("unit")
        require(isinstance(metric_name, str) and metric_name, f"inner metric name must be non-empty: {name}")
        require(metric_name not in metric_names, f"duplicate inner metric name: {metric_name}")
        metric_names.add(metric_name)
        require(
            isinstance(path, list) and path and all(isinstance(field, str) and field for field in path),
            f"inner metric path must be a non-empty list of strings: {metric_name}",
        )
        require(
            isinstance(key, list) and all(isinstance(field, str) and field for field in key),
            f"inner metric key must be a list of strings: {metric_name}",
        )
        require(unit is None or isinstance(unit, str), f"inner metric unit must be a string: {metric_name}")
        normalized_metrics.append({"name": metric_name, "path": path, "key": key, "unit": unit})
    expected_exit = workload.get("expected_exit", 0)
    require(isinstance(expected_exit, int), f"expected_exit must be an integer: {name}")
    compare_stderr = workload.get("compare_stderr", False)
    require(isinstance(compare_stderr, bool), f"compare_stderr must be boolean: {name}")
    environment = workload.get("environment", {})
    require(
        isinstance(environment, dict)
        and all(isinstance(key, str) and isinstance(value, str) for key, value in environment.items()),
        f"workload environment must map strings to strings: {name}",
    )
    normalized = dict(workload)
    normalized["name"] = name
    normalized["args"] = args
    normalized["files"] = [normalize_file(item) for item in files]
    normalized["outputs"] = normalized_outputs
    normalized["inner_metrics"] = normalized_metrics
    normalized["expected_exit"] = expected_exit
    normalized["compare_stderr"] = compare_stderr
    if semantic is not None:
        normalized["semantic"] = dict(semantic)
    return normalized

def workload_manifest_digest(
    workloads: Iterable[Mapping[str, Any]], environment: Mapping[str, str] | None = None
) -> str:
    identities = []
    for workload in workloads:
        identities.append(
            {
                "name": workload["name"],
                "args": workload["args"],
                "files": workload["files"],
                "outputs": workload["outputs"],
                "semantic": workload.get("semantic"),
                "inner_metrics": workload.get("inner_metrics", []),
                "expected_exit": workload["expected_exit"],
                "compare_stderr": workload["compare_stderr"],
                "environment": workload.get("environment", {}),
            }
        )
    return digest_bytes(canonical_json({"environment": environment or {}, "workloads": identities}))

def git_value(worktree: Path, *arguments: str) -> str:
    process = subprocess.run(
        ["git", "-C", str(worktree), *arguments],
        check=False,
        capture_output=True,
        text=True,
    )
    if process.returncode:
        detail = process.stderr.strip() or process.stdout.strip()
        raise RunnerError(f"git identity command failed in {worktree}: {detail}")
    return process.stdout.strip()

def verify_worktree(worktree_value: Path, revision: str) -> dict[str, Any]:
    require(REVISION_RE.fullmatch(revision) is not None, f"revision must be a full SHA-1: {revision}")
    worktree = worktree_value.resolve()
    require(worktree.is_dir(), f"worktree does not exist: {worktree}")
    top = Path(git_value(worktree, "rev-parse", "--show-toplevel")).resolve()
    require(top == worktree, f"worktree path is not its Git root: {worktree} (Git says {top})")
    head = git_value(worktree, "rev-parse", "HEAD")
    require(head == revision, f"worktree {worktree} is at {head}, requested {revision}")
    status = git_value(worktree, "status", "--porcelain=v1", "--untracked-files=all")
    require(not status, f"worktree is dirty: {worktree}\n{status}")
    tree = git_value(worktree, "rev-parse", "HEAD^{tree}")
    return {"path": str(worktree), "revision": head, "tree": tree, "status": "clean"}

def verify_binary(binary_value: Path) -> dict[str, Any]:
    require(binary_value.is_absolute(), f"binary path must be absolute: {binary_value}")
    binary = binary_value.resolve()
    require(binary.is_file(), f"binary does not exist: {binary}")
    require(os.access(binary, os.X_OK), f"binary is not executable: {binary}")
    return {
        "path": str(binary),
        "bytes": binary.stat().st_size,
        "sha256": digest_file(binary),
    }

def verify_identity(worktree_value: Path, revision: str, binary_value: Path) -> dict[str, Any]:
    identity = verify_worktree(worktree_value, revision)
    identity["binary"] = verify_binary(binary_value)
    return identity

def choose_time_command(value: str | None) -> Path | None:
    if value:
        path = Path(value).resolve()
        require(path.is_file() and os.access(path, os.X_OK), f"time command is not executable: {path}")
        return path
    system_time = Path("/usr/bin/time")
    if system_time.is_file() and os.access(system_time, os.X_OK):
        return system_time
    return None

def parse_time_metrics(path: Path) -> tuple[int | None, int | None, int | None, str | None]:
    if not path.is_file():
        return None, None, None, "time command produced no metrics file"
    fields = path.read_text(encoding="utf-8", errors="replace").strip().split()
    if len(fields) != 3:
        return None, None, None, f"time metrics had {len(fields)} fields, expected 3"
    try:
        user = int(Decimal(fields[0]) * Decimal(1_000_000_000))
        system = int(Decimal(fields[1]) * Decimal(1_000_000_000))
        rss = int(fields[2])
    except (InvalidOperation, ValueError) as error:
        return None, None, None, f"invalid time metrics: {error}"
    return user, system, rss, None

def kill_process_group(process: subprocess.Popen[bytes]) -> None:
    try:
        if os.name == "posix":
            os.killpg(process.pid, signal.SIGTERM)
        else:
            process.terminate()
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=1)
        return
    except subprocess.TimeoutExpired:
        pass
    try:
        if os.name == "posix":
            os.killpg(process.pid, signal.SIGKILL)
        else:
            process.kill()
    except ProcessLookupError:
        pass

def drop_ignored_keys(value: Any, ignored: set[str]) -> Any:
    if isinstance(value, dict):
        return {
            key: drop_ignored_keys(child, ignored)
            for key, child in value.items()
            if key not in ignored
        }
    if isinstance(value, list):
        return [drop_ignored_keys(child, ignored) for child in value]
    return value

def semantic_value(
    stdout: bytes,
    stderr: bytes,
    semantic: Mapping[str, Any] | None,
    inner_specs: Iterable[Mapping[str, Any]] = (),
) -> tuple[str, Any, dict[str, Any], str | None]:
    inner_specs = list(inner_specs)
    if semantic is None:
        if inner_specs:
            return digest_bytes(stdout), None, {}, "inner metrics require semantic JSON output"
        return digest_bytes(stdout), None, {}, None
    stream = stdout if semantic.get("stream", "stdout") == "stdout" else stderr
    try:
        text = stream.decode("utf-8")
        if semantic.get("format", "json") == "json":
            value = json.loads(text)
            records = [value]
        else:
            value = [json.loads(line) for line in text.splitlines() if line.strip()]
            records = value
        inner_metrics, metric_errors = extract_inner_metrics(records, inner_specs)
        ignored = set(semantic.get("ignore_keys", []))
        value = drop_ignored_keys(value, ignored)
        if "expected" in semantic and value != drop_ignored_keys(semantic["expected"], ignored):
            return "", value, inner_metrics, "semantic JSON differs from the manifest expectation"
        metric_error = "; ".join(metric_errors) if metric_errors else None
        return digest_bytes(canonical_json(value)), value, inner_metrics, metric_error
    except (UnicodeDecodeError, json.JSONDecodeError, TypeError, ValueError) as error:
        return "", None, {}, f"semantic JSON parse failed: {error}"

def output_identities(sample_dir: Path, outputs: Iterable[Mapping[str, Any]]) -> tuple[dict[str, Any], list[str]]:
    identities: dict[str, Any] = {}
    errors: list[str] = []
    for output in outputs:
        relative = Path(str(output["path"]))
        path = sample_dir / relative
        key = str(relative)
        if not path.is_file():
            identities[key] = {"exists": False}
            if output.get("required", True):
                errors.append(f"declared output is missing: {key}")
            continue
        actual = digest_file(path)
        identity = {"exists": True, "bytes": path.stat().st_size, "sha256": actual}
        expected = output.get("sha256")
        if expected is not None and actual != expected:
            errors.append(f"declared output hash mismatch: {key}")
        identities[key] = identity
    return identities, errors

def child_affinity(cpus: set[int] | None):
    if cpus is None:
        return None

    def set_affinity() -> None:
        os.sched_setaffinity(0, cpus)

    return set_affinity

def run_sample(
    *,
    side: str,
    binary: Path,
    workload: Mapping[str, Any],
    sample_dir: Path,
    environment: Mapping[str, str],
    timeout_seconds: float,
    time_command: Path | None,
    cpus: set[int] | None,
    max_output_bytes: int,
) -> dict[str, Any]:
    command = [str(binary), *[str(arg) for arg in workload["args"]]]
    metrics_path = sample_dir / "time.metrics"
    before = resource.getrusage(resource.RUSAGE_CHILDREN) if resource is not None else None
    timed_command = command
    if time_command is not None:
        timed_command = [str(time_command), "-f", TIME_FORMAT, "-o", str(metrics_path), "--", *command]
    child_environment = os.environ.copy()
    child_environment.update(environment)
    started = time.monotonic_ns()
    timed_out = False
    process: subprocess.Popen[bytes] | None = None
    stdout = b""
    stderr = b""
    try:
        process = subprocess.Popen(
            timed_command,
            cwd=sample_dir,
            env=child_environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            preexec_fn=child_affinity(cpus),
        )
        try:
            stdout, stderr = process.communicate(timeout=timeout_seconds)
        except subprocess.TimeoutExpired:
            timed_out = True
            kill_process_group(process)
            try:
                stdout, stderr = process.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                kill_process_group(process)
                stdout, stderr = process.communicate()
    except OSError as error:
        raise RunnerError(f"failed to execute {side} {workload['name']}: {error}") from error
    finally:
        ended = time.monotonic_ns()
    require(process is not None, "sample process was not created")
    after = resource.getrusage(resource.RUSAGE_CHILDREN) if resource is not None else None
    user_cpu_ns: int | None
    system_cpu_ns: int | None
    max_rss_kib: int | None
    metrics_error: str | None
    if time_command is not None:
        user_cpu_ns, system_cpu_ns, max_rss_kib, metrics_error = parse_time_metrics(metrics_path)
        metrics_source = "usr-bin-time" if metrics_error is None else None
        if metrics_error is not None and before is not None and after is not None:
            user_cpu_ns = max(0, int((after.ru_utime - before.ru_utime) * 1_000_000_000))
            system_cpu_ns = max(0, int((after.ru_stime - before.ru_stime) * 1_000_000_000))
            max_rss_kib = None
            metrics_source = "resource.getrusage"
    else:
        if before is not None and after is not None:
            user_cpu_ns = max(0, int((after.ru_utime - before.ru_utime) * 1_000_000_000))
            system_cpu_ns = max(0, int((after.ru_stime - before.ru_stime) * 1_000_000_000))
        else:
            user_cpu_ns = system_cpu_ns = None
        max_rss_kib = None
        metrics_error = "max RSS unavailable without /usr/bin/time"
        metrics_source = "resource.getrusage" if resource is not None else None
    semantic_digest, semantic_projection, inner_metrics, semantic_error = semantic_value(
        stdout, stderr, workload.get("semantic"), workload.get("inner_metrics", [])
    )
    errors: list[str] = []
    if timed_out:
        errors.append(f"process exceeded timeout of {timeout_seconds:g}s")
    if process.returncode != workload["expected_exit"]:
        errors.append(f"exit status {process.returncode}, expected {workload['expected_exit']}")
    if semantic_error is not None:
        errors.append(semantic_error)
    if len(stdout) > max_output_bytes or len(stderr) > max_output_bytes:
        errors.append(f"captured output exceeded {max_output_bytes} bytes")
    output_records, output_errors = output_identities(sample_dir, workload["outputs"])
    errors.extend(output_errors)
    record: dict[str, Any] = {
        "side": side,
        "workload": workload["name"],
        "argv": command,
        "timed_argv": timed_command,
        "cwd": str(sample_dir),
        "wall_ns": ended - started,
        "user_cpu_ns": user_cpu_ns,
        "system_cpu_ns": system_cpu_ns,
        "max_rss_kib": max_rss_kib,
        "resource_metrics": metrics_source,
        "resource_metrics_error": metrics_error,
        "returncode": process.returncode,
        "timed_out": timed_out,
        "stdout_bytes": len(stdout),
        "stdout_sha256": digest_bytes(stdout),
        "stderr_bytes": len(stderr),
        "stderr_sha256": digest_bytes(stderr),
        "semantic_sha256": semantic_digest,
        "semantic_preview": semantic_projection
        if semantic_projection is not None and len(canonical_json(semantic_projection)) <= MAX_SEMANTIC_PREVIEW_BYTES
        else None,
        "inner_metrics": inner_metrics,
        "outputs": output_records,
        "validation_errors": errors,
    }
    return record

def compare_sample(sample: Mapping[str, Any], reference: Mapping[str, Any], workload: Mapping[str, Any]) -> list[str]:
    errors = list(sample["validation_errors"])
    if sample["semantic_sha256"] != reference["semantic_sha256"]:
        errors.append("semantic output differs from baseline")
    if sample["outputs"] != reference["outputs"]:
        errors.append("declared output differs from baseline")
    if workload.get("compare_stderr", False) and sample["stderr_sha256"] != reference["stderr_sha256"]:
        errors.append("stderr differs from baseline")
    errors.extend(compare_inner_shapes(sample, reference))
    if errors != sample["validation_errors"]:
        sample["validation_errors"] = errors
    return errors

def statistic(values: Iterable[int]) -> dict[str, int]:
    ordered = sorted(values)
    if not ordered:
        return {"count": 0}
    middle = len(ordered) // 2
    if len(ordered) % 2:
        median = ordered[middle]
    else:
        median = (ordered[middle - 1] + ordered[middle]) // 2
    return {
        "count": len(ordered),
        "min": ordered[0],
        "median": median,
        "mean": sum(ordered) // len(ordered),
        "max": ordered[-1],
    }

def paired_statistics(pairs: Iterable[Mapping[str, Any]], metric: str) -> dict[str, Any]:
    deltas: list[int] = []
    ratios: list[float] = []
    for pair in pairs:
        baseline = pair.get("baseline", {}).get(metric)
        candidate = pair.get("candidate", {}).get(metric)
        if baseline is None or candidate is None:
            continue
        deltas.append(candidate - baseline)
        if baseline:
            ratios.append(candidate / baseline)
    result: dict[str, Any] = {"delta": statistic(deltas)}
    if ratios:
        result["ratio"] = {
            "count": len(ratios),
            "median": statistics.median(ratios),
            "mean": statistics.mean(ratios),
            "min": min(ratios),
            "max": max(ratios),
        }
    else:
        result["ratio"] = {"count": 0}
    return result

def make_pair(pair_index: int, order: str, baseline: Mapping[str, Any], candidate: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "record": "pair",
        "pair_index": pair_index,
        "order": order,
        "baseline_sample": baseline["sample_id"],
        "candidate_sample": candidate["sample_id"],
        "baseline": {metric: baseline[metric] for metric in METRICS},
        "candidate": {metric: candidate[metric] for metric in METRICS},
        "inner_metrics": pair_inner_metrics(
            baseline.get("inner_metrics", {}), candidate.get("inner_metrics", {})
        ),
    }

def summarize_workload(name: str, samples: list[Mapping[str, Any]], pairs: list[Mapping[str, Any]]) -> dict[str, Any]:
    measured = [sample for sample in samples if sample["role"] == "measure"]
    by_side = {
        side: [sample for sample in measured if sample["side"] == side]
        for side in ("baseline", "candidate")
    }
    side_summary: dict[str, Any] = {}
    for side, side_samples in by_side.items():
        side_summary[side] = {
            metric: statistic(
                sample[metric] for sample in side_samples if sample[metric] is not None
            )
            for metric in METRICS
        }
    return {
        "name": name,
        "warmup_count": sum(sample["role"] == "warmup" for sample in samples),
        "measured_count": len(measured),
        "pair_count": len(pairs),
        "sides": side_summary,
        "paired": {metric: paired_statistics(pairs, metric) for metric in METRICS},
        "inner": summarize_inner_metrics(samples, pairs),
    }

def host_identity() -> dict[str, Any]:
    affinity = None
    if hasattr(os, "sched_getaffinity"):
        affinity = sorted(os.sched_getaffinity(0))
    load = None
    try:
        load = list(os.getloadavg())
    except OSError:
        pass
    return {
        "platform": platform.platform(),
        "system": platform.system(),
        "release": platform.release(),
        "machine": platform.machine(),
        "python": platform.python_version(),
        "python_implementation": platform.python_implementation(),
        "processor": platform.processor(),
        "cpu_count": os.cpu_count(),
        "cpu_affinity": affinity,
        "load_average": load,
    }

def parse_cpu_list(value: str | None) -> set[int] | None:
    if value is None:
        return None
    require(hasattr(os, "sched_getaffinity"), "--cpu-list requires sched_getaffinity support")
    cpus: set[int] = set()
    try:
        for piece in value.split(","):
            if "-" in piece:
                first, last = (int(part) for part in piece.split("-", 1))
                require(first <= last, f"invalid CPU range: {piece}")
                cpus.update(range(first, last + 1))
            else:
                cpus.add(int(piece))
    except ValueError as error:
        raise RunnerError(f"invalid --cpu-list: {value}") from error
    require(cpus, "--cpu-list cannot be empty")
    allowed = set(os.sched_getaffinity(0))
    require(cpus <= allowed, f"requested CPUs {sorted(cpus)} are outside affinity {sorted(allowed)}")
    return cpus

def write_record(stream, record: Mapping[str, Any]) -> None:
    stream.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n")
    stream.flush()
