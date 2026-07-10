#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
benchmark_dir="$repo_root/benchmarks/plain-tex"
tfm_dir="$repo_root/crates/tex-fonts/tests/fixtures/cm"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="$repo_root/$target_dir"
fi
umber_bin="$target_dir/release/umber"
runs=5
inputs=(
  expand.tex
  paragraph-wide.tex
  paragraph-narrow.tex
  math.tex
  math-nested.tex
  pages.tex
  dvi.tex
)
engines=(umber)

for candidate in tex pdftex luatex xetex; do
  if command -v "$candidate" >/dev/null 2>&1; then
    engines+=("$candidate")
  fi
done

printf '%s\n' 'Building release Umber outside the timed region' >&2
cargo build --release -p umber >/dev/null

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/umber-plain-tex-bench.XXXXXX")"
results="$work_dir/results.tsv"
trap 'rm -rf "$work_dir"' EXIT

prepare_run() {
  local input="$1"
  local run_dir="$2"
  mkdir -p "$run_dir"
  cp "$benchmark_dir/$input" "$run_dir/$input"
  cp "$benchmark_dir/benchmark-preamble.inc" "$run_dir/benchmark-preamble.inc"
  cp "$tfm_dir"/*.tfm "$run_dir/"
}

artifact_path() {
  local engine="$1"
  local stem="$2"
  local run_dir="$3"
  if [[ "$engine" == xetex ]]; then
    printf '%s/%s.xdv\n' "$run_dir" "$stem"
  else
    printf '%s/%s.dvi\n' "$run_dir" "$stem"
  fi
}

invoke_engine() {
  local engine="$1"
  local input="$2"
  local run_dir="$3"
  local stem="${input%.tex}"
  (
    cd "$run_dir"
    case "$engine" in
      umber)
        "$umber_bin" run --dvi "$stem.dvi" "$input" >stdout 2>stderr
        ;;
      tex)
        tex -interaction=batchmode "$input" >stdout 2>stderr
        ;;
      pdftex)
        pdftex -interaction=batchmode -output-format=dvi "$input" >stdout 2>stderr
        ;;
      luatex)
        luatex --interaction=batchmode --output-format=dvi "$input" >stdout 2>stderr
        ;;
      xetex)
        xetex -interaction=batchmode -no-pdf "$input" >stdout 2>stderr
        ;;
      *)
        printf 'bench-plain-tex: unsupported engine: %s\n' "$engine" >&2
        return 2
        ;;
    esac
  )
}

report_failure() {
  local engine="$1"
  local input="$2"
  local run_dir="$3"
  local stem="${input%.tex}"
  printf 'bench-plain-tex: %s failed on %s\n' "$engine" "$input" >&2
  for transcript in "$run_dir/stderr" "$run_dir/stdout" "$run_dir/$stem.log"; do
    if [[ -s "$transcript" ]]; then
      printf '%s\n' "--- ${transcript##*/} (tail) ---" >&2
      tail -20 "$transcript" >&2
    fi
  done
}

expected_marker() {
  case "$1" in
    expand) printf '%s\n' 'BENCH expand 0,100000' ;;
    paragraph-wide) printf '%s\n' 'BENCH pwide 2000' ;;
    paragraph-narrow) printf '%s\n' 'BENCH pnarrow 1000' ;;
    math) printf '%s\n' 'BENCH math 20000' ;;
    math-nested) printf '%s\n' 'BENCH math-nested 10000' ;;
    pages) printf '%s\n' 'BENCH pages 502' ;;
    dvi) printf '%s\n' 'BENCH dvi 1000,48' ;;
    *) return 1 ;;
  esac
}

validate_run() {
  local engine="$1"
  local input="$2"
  local run_dir="$3"
  local stem="${input%.tex}"
  local artifact
  local marker
  artifact="$(artifact_path "$engine" "$stem" "$run_dir")"
  marker="$(expected_marker "$stem")"
  if [[ ! -s "$artifact" ]]; then
    printf 'bench-plain-tex: %s produced no artifact for %s\n' \
      "$engine" "$input" >&2
    return 1
  fi
  if ! rg -Fq "$marker" "$run_dir/stdout" "$run_dir/$stem.log" 2>/dev/null; then
    printf 'bench-plain-tex: %s has the wrong completion marker for %s\n' \
      "$engine" "$input" >&2
    return 1
  fi
}

normalized_artifact_checksum() {
  local artifact="$1"
  local comment_length
  comment_length="$(od -An -tu1 -j14 -N1 "$artifact" | tr -d '[:space:]')"
  if [[ -z "$comment_length" ]]; then
    printf 'bench-plain-tex: cannot read preamble from %s\n' "$artifact" >&2
    return 1
  fi
  {
    dd if="$artifact" bs=1 count=15 2>/dev/null
    tail -c "+$((16 + comment_length))" "$artifact"
  } | cksum
}

measure_engine() {
  local engine="$1"
  local input="$2"
  local stem="${input%.tex}"
  local warm_dir="$work_dir/$stem-$engine-warm"
  local expected_checksum
  local times=""
  local failures=0

  prepare_run "$input" "$warm_dir"
  if ! invoke_engine "$engine" "$input" "$warm_dir"; then
    if [[ "$engine" != umber ]]; then
      report_failure "$engine" "$input" "$warm_dir"
      return 1
    fi
  fi
  if [[ "$engine" != umber ]]; then
    validate_run "$engine" "$input" "$warm_dir"
    expected_checksum="$(normalized_artifact_checksum \
      "$(artifact_path "$engine" "$stem" "$warm_dir")")"
  fi
  rm -rf "$warm_dir"

  for run in $(seq 1 "$runs"); do
    local run_dir="$work_dir/$stem-$engine-$run"
    local seconds
    local actual_checksum
    local status=0
    prepare_run "$input" "$run_dir"
    seconds="$({ TIMEFORMAT='%R'; time invoke_engine "$engine" "$input" "$run_dir"; } 2>&1)" || status=$?
    if [[ "$status" -ne 0 && "$engine" != umber ]]; then
      report_failure "$engine" "$input" "$run_dir"
      return 1
    fi
    if [[ "$engine" == umber ]]; then
      if [[ "$status" -ne 0 ]]; then
        failures=$((failures + 1))
      fi
    else
      validate_run "$engine" "$input" "$run_dir"
      actual_checksum="$(normalized_artifact_checksum \
        "$(artifact_path "$engine" "$stem" "$run_dir")")"
      if [[ "$actual_checksum" != "$expected_checksum" ]]; then
        printf 'bench-plain-tex: %s produced nondeterministic output for %s run %d\n' \
          "$engine" "$input" "$run" >&2
        return 1
      fi
    fi
    times="$times $seconds"
    rm -rf "$run_dir"
  done

  printf '%s\n' "$times" | awk \
    -v workload="$stem" -v engine="$engine" -v failures="$failures" '
    {
      min = $1
      max = $1
      for (i = 1; i <= NF; i++) {
        sum += $i
        if ($i < min) min = $i
        if ($i > max) max = $i
      }
      status = failures == 0 ? "ok" : "error(" failures ")"
      printf "%s\t%s\t%.6f\t%.6f\t%.6f\t%s\n", \
        workload, engine, sum / NF, min, max, status
    }
  ' >>"$results"
}

baseline=umber
for engine in "${engines[@]}"; do
  if [[ "$engine" == tex ]]; then
    baseline=tex
    break
  elif [[ "$baseline" == umber ]]; then
    baseline="$engine"
  fi
done

printf 'Detected engines:' >&2
for engine in "${engines[@]}"; do
  printf ' %s' "$engine" >&2
done
printf '\nOne warm-up and %d measured runs per engine/workload.\n' "$runs" >&2

for input in "${inputs[@]}"; do
  for engine in "${engines[@]}"; do
    printf 'Benchmarking %-18s with %s\n' "${input%.tex}" "$engine" >&2
    measure_engine "$engine" "$input"
  done
done

awk -F '\t' -v baseline="$baseline" '
  {
    workload[NR] = $1
    engine[NR] = $2
    mean[NR] = $3
    min[NR] = $4
    max[NR] = $5
    status[NR] = $6
    if ($2 == baseline) base[$1] = $3
    rows = NR
  }
  END {
    printf "\n%-20s %-9s %10s %10s %10s %10s %10s\n", \
      "benchmark", "engine", "mean (s)", "min (s)", "max (s)", "vs " baseline, "status"
    for (i = 1; i <= rows; i++) {
      ratio = mean[i] / base[workload[i]]
      printf "%-20s %-9s %10.3f %10.3f %10.3f %9.2fx %10s\n", \
        workload[i], engine[i], mean[i], min[i], max[i], ratio, status[i]
    }
  }
' "$results"
