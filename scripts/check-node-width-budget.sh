#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/benchmarks/tex-exec/Cargo.toml"
baseline="$repo_root/benchmarks/tex-exec/node-width-budgets.json"
target_dir="${CARGO_TARGET_DIR:-$repo_root/benchmarks/tex-exec/target}"

cargo bench --manifest-path "$manifest" --bench widths -- --noplot

python3 - "$baseline" "$target_dir/criterion/hpack_widths" <<'PY'
import json
import pathlib
import sys

baseline_path = pathlib.Path(sys.argv[1])
criterion_root = pathlib.Path(sys.argv[2])
baseline = json.loads(baseline_path.read_text())
tolerance = baseline["regression_tolerance_percent"] / 100.0
failed = False
for name, expected_ns in baseline["benchmarks"].items():
    estimates = criterion_root / name / "new" / "estimates.json"
    measured_ns = json.loads(estimates.read_text())["mean"]["point_estimate"]
    limit_ns = expected_ns * (1.0 + tolerance)
    status = "ok" if measured_ns <= limit_ns else "REGRESSION"
    print(f"{name}: {measured_ns:.3f} ns (budget {limit_ns:.3f} ns) {status}")
    failed |= measured_ns > limit_ns
if failed:
    raise SystemExit("compact node width performance budget exceeded")
PY
