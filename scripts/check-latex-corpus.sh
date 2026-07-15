#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
texmf_dist="${UMBER_TEXMF_DIST:-/usr/local/texlive/2025/texmf-dist}"
reference_latex="${UMBER_REF_LATEX:-$(command -v latex || true)}"
source_date_epoch="$(awk '$1 == "source_date_epoch" { print $2 }' "${repo_root}/tests/latex-source.lock")"
runtime_lock="${repo_root}/tests/latex-runtime.lock"

fail() {
  printf 'check-latex-corpus.sh: %s\n' "$*" >&2
  exit 1
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

[[ -x "$reference_latex" ]] || fail "missing reference LaTeX; set UMBER_REF_LATEX"
"$reference_latex" --version | head -1 | grep -q 'TeX Live 2025' || \
  fail "reference LaTeX is not from pinned TeX Live 2025"

cd "$repo_root"
scripts/build-latex-format.sh --texmf-dist "$texmf_dist"
cargo build --release -p umber
cargo build -p refexec

umber_bin="${CARGO_TARGET_DIR:-${repo_root}/target}/release/umber"
refexec_bin="${CARGO_TARGET_DIR:-${repo_root}/target}/debug/refexec"
format_file="${repo_root}/target/latex-format/latex.fmt"
texinputs="${texmf_dist}/tex/latex/base:${texmf_dist}/tex/latex/l3kernel:${texmf_dist}/tex/latex/l3backend:${texmf_dist}/tex/generic/unicode-data:${texmf_dist}/tex/generic/babel:${texmf_dist}/tex/generic/hyphen"
texfonts="${texmf_dist}/fonts/tfm/public/cm:${texmf_dist}/fonts/tfm/public/latex-fonts:${texmf_dist}/fonts/tfm/jknappen/ec"
tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-latex-corpus.XXXXXX")"
cleanup() {
  local status=$?
  if [[ $status -eq 0 ]]; then
    rm -rf "$tmp_root"
  else
    printf 'check-latex-corpus.sh: failed artifacts: %s\n' "$tmp_root" >&2
  fi
}
trap cleanup EXIT

expected_runtime="${tmp_root}/expected-runtime.inputs"
actual_runtime="${tmp_root}/actual-runtime.inputs"
: > "$expected_runtime"
: > "$actual_runtime"
while read -r record kind relative expected_bytes expected_hash extra; do
  [[ -z "${record:-}" || "$record" == \#* ]] && continue
  [[ "$record" == source ]] || continue
  [[ -z "${extra:-}" ]] || fail "invalid runtime closure entry for $relative"
  source="${texmf_dist}/${relative}"
  [[ -f "$source" ]] || fail "missing pinned runtime input: $source"
  actual_bytes="$(wc -c < "$source" | tr -d ' ')"
  [[ "$actual_bytes" == "$expected_bytes" ]] || fail "runtime length mismatch for $relative"
  [[ "$(sha256 "$source")" == "$expected_hash" ]] || fail "runtime hash mismatch for $relative"
  printf '%s\t%s\n' "$expected_bytes" "$source" >> "$expected_runtime"
done < "$runtime_lock"
LC_ALL=C sort -u -o "$expected_runtime" "$expected_runtime"

compare_auxiliary_files() {
  local reference_dir="$1"
  local umber_dir="$2"
  local extension
  for extension in aux toc lof lot out; do
    local reference_file="${reference_dir}/document.${extension}"
    local umber_file="${umber_dir}/document.${extension}"
    if [[ -e "$reference_file" || -e "$umber_file" ]]; then
      [[ -f "$reference_file" && -f "$umber_file" ]] || \
        fail "auxiliary file-set mismatch for document.${extension}"
      cmp "$reference_file" "$umber_file" || \
        fail "auxiliary mismatch for document.${extension}"
    fi
  done
}

for source in tests/latex/article.tex tests/latex/report.tex tests/latex/book.tex tests/latex/letter.tex; do
  case_name="$(basename "$source" .tex)"
  reference_dir="${tmp_root}/${case_name}/reference"
  umber_dir="${tmp_root}/${case_name}/umber"
  mkdir -p "$reference_dir" "$umber_dir"
  cp "$source" "${reference_dir}/document.tex"
  cp "$source" "${umber_dir}/document.tex"
  cp "$format_file" "${umber_dir}/latex.fmt"

  for pass in 1 2 3; do
    (
      cd "$reference_dir"
      env SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
        "$reference_latex" -interaction=batchmode document.tex >/dev/null
    ) || fail "reference LaTeX failed for ${case_name}, pass ${pass}"
    (
      cd "$umber_dir"
      env SOURCE_DATE_EPOCH="$source_date_epoch" TEXINPUTS="$texinputs" TEXFONTS="$texfonts" \
        "$umber_bin" run --latex document.tex --format latex.fmt --dvi document.dvi \
          --input-records-out document.inputs \
          > document.stdout 2> document.stderr
    ) || fail "Umber failed for ${case_name}, pass ${pass}"
    if grep -q '^! ' "${umber_dir}/document.stdout"; then
      grep -m1 '^! ' "${umber_dir}/document.stdout" >&2
      fail "Umber emitted a diagnostic for ${case_name}, pass ${pass}"
    fi
  done

  "$refexec_bin" --compare-existing-dvi \
    "${reference_dir}/document.dvi" "${umber_dir}/document.dvi" || \
    fail "DVI mismatch for ${case_name}"
  compare_auxiliary_files "$reference_dir" "$umber_dir"
  awk -F '\t' -v prefix="${texmf_dist}/" 'index($2, prefix) == 1' \
    "${umber_dir}/document.inputs" >> "$actual_runtime"
  printf 'LaTeX corpus parity: %s (3 passes)\n' "$case_name"
done

LC_ALL=C sort -u -o "$actual_runtime" "$actual_runtime"
cmp "$expected_runtime" "$actual_runtime" || fail "base-corpus runtime closure changed"
printf 'LaTeX runtime closure: exact (%s pinned inputs)\n' "$(wc -l < "$actual_runtime" | tr -d ' ')"
