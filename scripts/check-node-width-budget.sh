#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/benchmarks/tex-typeset/Cargo.toml"
baseline="$repo_root/benchmarks/tex-typeset/node-width-budgets.json"
target_dir="${CARGO_TARGET_DIR:-$repo_root/benchmarks/tex-typeset/target}"

set +e
python3 - "$baseline" <<'PY'
import json
import math
import pathlib
import re
import subprocess
import sys

EXPECTED_ROWS = {"same_font_64", "same_font_4096", "mixed_4096"}
REQUIRED_KEYS = {
    "schema_version",
    "profile",
    "host",
    "rustc_release",
    "regression_tolerance_percent",
    "benchmarks",
}


def report(status, **fields):
    print(json.dumps({"status": status, **fields}, sort_keys=True))


def fail(message):
    report("invalid_baseline", message=message)
    raise SystemExit(1)


baseline_path = pathlib.Path(sys.argv[1])
try:
    baseline = json.loads(baseline_path.read_text())
except (OSError, json.JSONDecodeError) as error:
    fail(f"cannot read baseline metadata: {error}")

if not isinstance(baseline, dict):
    fail("top-level value must be an object")
if set(baseline) != REQUIRED_KEYS:
    fail(
        "top-level keys must be exactly "
        + ", ".join(sorted(REQUIRED_KEYS))
    )
if baseline["schema_version"] != 1:
    fail("schema_version must be 1")
for key in ("profile", "host", "rustc_release"):
    if not isinstance(baseline[key], str) or not baseline[key]:
        fail(f"{key} must be a non-empty string")
if not re.fullmatch(r"[A-Za-z0-9_.+-]+", baseline["host"]):
    fail("host is not a valid Rust target triple")
if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?", baseline["rustc_release"]):
    fail("rustc_release must be an exact release such as 1.93.0")

tolerance = baseline["regression_tolerance_percent"]
if isinstance(tolerance, bool) or not isinstance(tolerance, (int, float)):
    fail("regression_tolerance_percent must be a number")
if not math.isfinite(tolerance) or tolerance < 0:
    fail("regression_tolerance_percent must be finite and non-negative")

benchmarks = baseline["benchmarks"]
if not isinstance(benchmarks, dict) or set(benchmarks) != EXPECTED_ROWS:
    fail("benchmark rows must be exactly " + ", ".join(sorted(EXPECTED_ROWS)))
for name, expected_ns in benchmarks.items():
    if isinstance(expected_ns, bool) or not isinstance(expected_ns, (int, float)):
        fail(f"benchmark {name} must have a numeric mean")
    if not math.isfinite(expected_ns) or expected_ns <= 0:
        fail(f"benchmark {name} mean must be finite and positive")

try:
    rustc = subprocess.run(
        ["rustc", "-vV"],
        check=True,
        capture_output=True,
        text=True,
    )
except (OSError, subprocess.CalledProcessError) as error:
    report("environment_error", message=f"cannot query rustc identity: {error}")
    raise SystemExit(1)

identity = {}
for line in rustc.stdout.splitlines():
    if ": " in line:
        key, value = line.split(": ", 1)
        if key in {"host", "release"}:
            if key in identity:
                report("environment_error", message=f"rustc reported duplicate {key}")
                raise SystemExit(1)
            identity[key] = value
if set(identity) != {"host", "release"}:
    report("environment_error", message="rustc -vV omitted host or release")
    raise SystemExit(1)

expected = {"host": baseline["host"], "rustc_release": baseline["rustc_release"]}
actual = {"host": identity["host"], "rustc_release": identity["release"]}
if actual != expected:
    report("unsupported", actual=actual, expected=expected, gating=False)
    raise SystemExit(4)
report("applicable", actual=actual, expected=expected, gating=True)
PY
qualification_status=$?
set -e
if ((qualification_status != 0)); then
  exit "$qualification_status"
fi

cargo bench --manifest-path "$manifest" --bench widths -- --noplot

python3 - "$baseline" "$target_dir/criterion/hpack_widths" <<'PY'
import json
import math
import pathlib
import sys

baseline_path = pathlib.Path(sys.argv[1])
criterion_root = pathlib.Path(sys.argv[2])
baseline = json.loads(baseline_path.read_text())
tolerance = baseline["regression_tolerance_percent"] / 100.0
failed = False
for name, expected_ns in baseline["benchmarks"].items():
    estimates = criterion_root / name / "new" / "estimates.json"
    try:
        measured_ns = json.loads(estimates.read_text())["mean"]["point_estimate"]
    except (OSError, json.JSONDecodeError, KeyError, TypeError) as error:
        print(json.dumps({
            "status": "invalid_results",
            "benchmark": name,
            "message": str(error),
        }, sort_keys=True))
        raise SystemExit(1)
    if (
        isinstance(measured_ns, bool)
        or not isinstance(measured_ns, (int, float))
        or not math.isfinite(measured_ns)
        or measured_ns <= 0
    ):
        print(json.dumps({
            "status": "invalid_results",
            "benchmark": name,
            "message": "mean point_estimate must be finite and positive",
        }, sort_keys=True))
        raise SystemExit(1)
    limit_ns = expected_ns * (1.0 + tolerance)
    status = "ok" if measured_ns <= limit_ns else "REGRESSION"
    print(f"{name}: {measured_ns:.3f} ns (budget {limit_ns:.3f} ns) {status}")
    failed |= measured_ns > limit_ns
if failed:
    print(json.dumps({"status": "regression", "gating": True}, sort_keys=True))
    raise SystemExit(1)
print(json.dumps({"status": "pass", "gating": True}, sort_keys=True))
PY
