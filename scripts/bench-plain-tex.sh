#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
benchmark_dir="$repo_root/benchmarks/plain-tex"
tex_bin="${TEX_BIN:-tex}"
runs=5
inputs=(
  expand.tex
  paragraph-wide.tex
  paragraph-narrow.tex
  math.tex
  pages.tex
  dvi.tex
)

if ! command -v "$tex_bin" >/dev/null 2>&1; then
  printf 'bench-plain-tex: TeX binary not found: %s\n' "$tex_bin" >&2
  exit 1
fi

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/umber-plain-tex-bench.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

run_one() {
  local input="$1"
  local stem="${input%.tex}"
  local run_dir="$2"
  mkdir -p "$run_dir"
  cp "$benchmark_dir/$input" "$run_dir/$input"
  if ! (
    cd "$run_dir"
    "$tex_bin" -interaction=batchmode "$input" >/dev/null
  ); then
    printf 'bench-plain-tex: %s failed; transcript tail follows\n' "$input" >&2
    if [[ -f "$run_dir/$stem.log" ]]; then
      tail -20 "$run_dir/$stem.log" >&2
    fi
    return 1
  fi
  if [[ ! -s "$run_dir/$stem.dvi" ]]; then
    printf 'bench-plain-tex: %s did not produce a nonempty DVI\n' "$input" >&2
    return 1
  fi
  case "$stem" in
    expand) expected='BENCHMARK expand checksum=0, iterations=500000' ;;
    paragraph-wide) expected='BENCHMARK paragraph-wide paragraphs=9000' ;;
    paragraph-narrow) expected='BENCHMARK paragraph-narrow paragraphs=4500' ;;
    math) expected='BENCHMARK math width=' ;;
    pages) expected='BENCHMARK pages pages=6001' ;;
    dvi) expected='BENCHMARK dvi pages=6000, lines-per-page=48' ;;
    *)
      printf 'bench-plain-tex: no expected marker for %s\n' "$input" >&2
      return 1
      ;;
  esac
  if ! rg -Fq "$expected" "$run_dir/$stem.log"; then
    printf 'bench-plain-tex: %s has the wrong completion marker\n' "$input" >&2
    return 1
  fi
}

normalized_dvi_checksum() {
  local dvi="$1"
  local comment_length
  comment_length="$(od -An -tu1 -j14 -N1 "$dvi" | tr -d '[:space:]')"
  if [[ -z "$comment_length" ]]; then
    printf 'bench-plain-tex: cannot read DVI preamble from %s\n' "$dvi" >&2
    return 1
  fi
  {
    dd if="$dvi" bs=1 count=15 2>/dev/null
    tail -c "+$((16 + comment_length))" "$dvi"
  } | cksum
}

printf 'Plain TeX benchmarks: %s (one warm-up, %d measured runs)\n' "$tex_bin" "$runs"
for input in "${inputs[@]}"; do
  stem="${input%.tex}"
  run_one "$input" "$work_dir/$stem-warmup"
  expected_checksum="$(normalized_dvi_checksum "$work_dir/$stem-warmup/$stem.dvi")"
  rm -rf "$work_dir/$stem-warmup"
  printf '%-24s' "$stem"
  for run in $(seq 1 "$runs"); do
    seconds="$({ TIMEFORMAT='%R'; time run_one "$input" "$work_dir/$stem-$run"; } 2>&1)"
    actual_checksum="$(normalized_dvi_checksum "$work_dir/$stem-$run/$stem.dvi")"
    if [[ "$actual_checksum" != "$expected_checksum" ]]; then
      printf '\nbench-plain-tex: %s produced nondeterministic DVI on run %d\n' \
        "$input" "$run" >&2
      exit 1
    fi
    rm -rf "$work_dir/$stem-$run"
    printf ' %s' "$seconds"
  done
  printf '\n'
done
