#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
benchmark_dir="$repo_root/benchmarks/plain-tex"
tfm_dir="$repo_root/crates/tex-fonts/tests/fixtures/cm"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="$repo_root/$target_dir"
fi
umber_bin="$target_dir/profiling/umber"
runs="${MEASURE_RUNS:-1}"
inputs=(
  expand.tex
  paragraph-wide.tex
  paragraph-narrow.tex
  math.tex
  math-nested.tex
  pages.tex
  dvi.tex
)

cargo build --profile profiling -p umber --features profiling-stats

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/umber-allocation-owners.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

for input in "${inputs[@]}"; do
  expected_hash=""
  for ((sample = 1; sample <= runs; sample++)); do
    run_dir="$work_dir/${input%.tex}-$sample"
    mkdir -p "$run_dir"
    cp "$benchmark_dir/$input" "$run_dir/"
    cp "$benchmark_dir/benchmark-preamble.inc" "$run_dir/"
    cp "$tfm_dir"/*.tfm "$run_dir/"
    (
      cd "$run_dir"
      /usr/bin/time -l "$umber_bin" run --profiling-stats --dvi output.dvi "$input" \
        >stdout 2>measurement
    )
    artifact_hash="$(shasum -a 256 "$run_dir/output.dvi" | awk '{print $1}')"
    if [[ -n "$expected_hash" && "$artifact_hash" != "$expected_hash" ]]; then
      printf 'non-deterministic DVI for %s: %s != %s\n' \
        "$input" "$artifact_hash" "$expected_hash" >&2
      exit 1
    fi
    expected_hash="$artifact_hash"
    printf 'ALLOCATION_SAMPLE workload=%s sample=%d dvi_sha256=%s\n' \
      "$input" "$sample" "$artifact_hash"
    grep -E \
      '^(ALLOC_|NODE_SURVIVOR|NODE_MEMORY_TOTAL|NODE_STORAGE_PEAK )|real |maximum resident set size|peak memory footprint' \
      "$run_dir/measurement"
  done
done
