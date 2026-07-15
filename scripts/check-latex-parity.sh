#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="${repo_root}/tests/latex-parity-manifest.txt"
source_dir="${repo_root}/third_party/latex2e-parity/source"
case_list="${repo_root}/third_party/latex2e-parity/dvi-cases.txt"
texmf_dist="${UMBER_TEXMF_DIST:-/usr/local/texlive/2025/texmf-dist}"
reference_latex="${UMBER_REF_LATEX:-$(command -v latex || true)}"
format_builder="${UMBER_LATEX_FORMAT_BUILDER:-${repo_root}/scripts/build-latex-format.sh}"
format_file=""
case_filter=""
offline=0
keep_work=0
self_test=0
format_build_count=0

target_dir="${CARGO_TARGET_DIR:-${repo_root}/target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="${repo_root}/${target_dir}"
fi
format_output_dir="${target_dir}/latex-parity/format"
receipt="${target_dir}/latex-parity/last-run-format-receipt.txt"
active_receipt="$receipt"
triage_dir="${target_dir}/latex-parity/triage"

usage() {
  cat <<'EOF'
usage: scripts/check-latex-parity.sh [options]

Options:
  --format PATH       Reuse an existing pregenerated Umber latex.fmt.
  --case NAME         Run one derived case name or repository-relative path.
  --offline           Do not fetch the pinned upstream LaTeX2e snapshot.
  --keep-work         Preserve successful reference and Umber work directories.
  --self-test-format-reuse
                      Test the build-once/stage-identically invariant only.

Without --format, the runner builds and verifies latex.fmt exactly once before
starting any case. Every case gets an isolated copy of those exact bytes and
loads it with `umber run --format latex.fmt`.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --format)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --format' >&2; exit 2; }
      format_file="$2"
      shift 2
      ;;
    --case)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing name after --case' >&2; exit 2; }
      case_filter="$2"
      shift 2
      ;;
    --offline) offline=1; shift ;;
    --keep-work) keep_work=1; shift ;;
    --self-test-format-reuse) self_test=1; shift ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'check-latex-parity.sh: unknown option: %s\n' "$1" >&2; exit 2 ;;
  esac
done

fail() {
  printf 'check-latex-parity.sh: %s\n' "$*" >&2
  exit 1
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

absolute_file() {
  local path="$1"
  local directory
  directory="$(cd "$(dirname "$path")" && pwd)"
  printf '%s/%s\n' "$directory" "$(basename "$path")"
}

prepare_format() {
  if [[ -z "$format_file" ]]; then
    mkdir -p "$format_output_dir"
    "$format_builder" --texmf-dist "$texmf_dist" --output-dir "$format_output_dir"
    format_build_count=$((format_build_count + 1))
    format_file="${format_output_dir}/latex.fmt"
  fi
  [[ -f "$format_file" ]] || fail "missing pregenerated format: $format_file"
  format_file="$(absolute_file "$format_file")"
  format_sha256="$(sha256 "$format_file")"
  format_bytes="$(wc -c < "$format_file" | tr -d ' ')"
}

start_receipt() {
  mkdir -p "$(dirname "$receipt")"
  active_receipt="${receipt}.$$"
  cat > "$active_receipt" <<EOF
schema 1
format ${format_file}
format_bytes ${format_bytes}
format_sha256 ${format_sha256}
builder_invocations ${format_build_count}
EOF
}

stage_format() {
  local case_name="$1"
  local directory="$2"
  local staged="${directory}/latex.fmt"
  cp "$format_file" "$staged"
  local staged_hash
  staged_hash="$(sha256 "$staged")"
  [[ "$staged_hash" == "$format_sha256" ]] || \
    fail "staged format identity changed for $case_name"
  printf 'case %s %s %s\n' "$case_name" "$staged_hash" "$staged" >> "$active_receipt"
}

run_format_reuse_self_test() {
  local temp
  temp="$(mktemp -d "${TMPDIR:-/tmp}/umber-latex-parity-self-test.XXXXXX")"
  trap "rm -rf '$temp'" EXIT
  local count_file="${temp}/builder-count"
  local stub_builder="${temp}/format-builder"
  cat > "$stub_builder" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output_dir=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-dir) output_dir="$2"; shift 2 ;;
    --texmf-dist) shift 2 ;;
    *) exit 2 ;;
  esac
done
count=0
[[ ! -f "$UMBER_SELF_TEST_COUNT_FILE" ]] || count="$(cat "$UMBER_SELF_TEST_COUNT_FILE")"
printf '%s\n' "$((count + 1))" > "$UMBER_SELF_TEST_COUNT_FILE"
mkdir -p "$output_dir"
printf 'one immutable pregenerated format\n' > "${output_dir}/latex.fmt"
EOF
  chmod +x "$stub_builder"
  export UMBER_SELF_TEST_COUNT_FILE="$count_file"
  format_builder="$stub_builder"
  format_output_dir="${temp}/format"
  receipt="${temp}/receipt.txt"
  format_file=""
  prepare_format
  start_receipt
  for name in first second third; do
    mkdir -p "${temp}/${name}"
    stage_format "$name" "${temp}/${name}"
  done
  [[ "$(cat "$count_file")" == 1 ]] || fail "self-test format builder did not run once"
  [[ "$format_build_count" == 1 ]] || fail "self-test recorded the wrong build count"
  [[ "$(awk '$1 == "case" { print $3 }' "$active_receipt" | sort -u | wc -l | tr -d ' ')" == 1 ]] || \
    fail "self-test staged more than one format identity"
  [[ "$(awk '$1 == "case" { count++ } END { print count + 0 }' "$active_receipt")" == 3 ]] || \
    fail "self-test did not stage every case"
  printf '%s\n' 'LaTeX parity format-reuse self-test: passed (one build, three identical restores)'
}

if [[ $self_test -eq 1 ]]; then
  run_format_reuse_self_test
  exit 0
fi

[[ -f "$manifest" ]] || fail "missing parity manifest: $manifest"
if [[ $offline -eq 1 ]]; then
  "${repo_root}/scripts/setup-latex-parity-tests.sh" --offline
else
  "${repo_root}/scripts/setup-latex-parity-tests.sh"
fi
[[ -x "$reference_latex" ]] || fail "missing reference LaTeX; set UMBER_REF_LATEX"
reference_version="$($reference_latex --version | sed -n '1p')"
[[ "$reference_version" == *'TeX Live 2025'* ]] || \
  fail "reference LaTeX is not from pinned TeX Live 2025: $reference_version"
[[ -d "$texmf_dist" ]] || fail "missing pinned texmf-dist root: $texmf_dist"

cd "$repo_root"
prepare_format
start_receipt
cargo build --release -p umber
cargo build -p parity-harness

umber_bin="${target_dir}/release/umber"
parity_bin="${target_dir}/debug/parity-harness"
[[ -x "$umber_bin" && -x "$parity_bin" ]] || fail "required parity binaries were not built"
source_date_epoch="$(awk '$1 == "source_date_epoch" { print $2 }' "$manifest")"
texinput_rel_dirs=(
  tex/latex/base tex/latex/tools tex/latex/graphics tex/latex/graphics-def
  tex/latex/amsmath tex/latex/amscls tex/latex/amsfonts
  tex/latex/l3kernel tex/latex/l3backend tex/latex/l3packages/xparse
  tex/latex/alegreya tex/latex/algolrevived tex/latex/cyrillic tex/latex/etoolbox
  tex/latex/hycolor tex/latex/hypdoc tex/latex/hyperref tex/latex/kvoptions
  tex/latex/kvsetkeys tex/latex/lm tex/latex/pict2e tex/latex/refcount
  tex/latex/rerunfilecheck tex/latex/stix2-type1 tex/latex/url
  tex/generic/babel tex/generic/hyphen tex/generic/unicode-data
  tex/generic/bigintcalc tex/generic/bitset tex/generic/gettitlestring
  tex/generic/iftex tex/generic/infwarerr tex/generic/intcalc
  tex/generic/kvdefinekeys tex/generic/ltxcmds tex/generic/pdfescape
  tex/generic/pdftexcmds tex/generic/stringenc tex/generic/uniquecounter
)
texinputs="."
for relative_dir in "${texinput_rel_dirs[@]}"; do
  texinputs+=":${texmf_dist}/${relative_dir}"
done
texfonts="${texmf_dist}/fonts/tfm/public/cm:${texmf_dist}/fonts/tfm/public/latex-fonts:${texmf_dist}/fonts/tfm/jknappen/ec"
work_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-latex-parity.XXXXXX")"
cleanup() {
  local status=$?
  if [[ $status -eq 0 && $keep_work -eq 0 ]]; then
    rm -rf "$work_root"
  else
    printf 'LaTeX parity work directory: %s\n' "$work_root" >&2
  fi
}
trap cleanup EXIT

case_error() {
  local case_name="$1"
  shift
  printf 'LaTeX DVI parity failed: %s: %s\n' "$case_name" "$*" >&2
}

run_one_case() {
  local path="$1"
  local case_name="$2"
  local case_root="${work_root}/${case_name}"
  local reference_dir="${case_root}/reference"
  local umber_dir="${case_root}/umber"
  local source_path="${source_dir}/${path}"
  local source_parent
  source_parent="$(dirname "$source_path")"
  local local_inputs=".:${source_parent}:${source_dir}/support:${source_dir}/base:${source_dir}/required/tools:${source_dir}/required/graphics:${source_dir}/required/amsmath:${texinputs}"
  mkdir -p "$reference_dir" "$umber_dir"
  cp "$source_path" "${reference_dir}/document.tex" || {
    case_error "$case_name" "could not stage reference source"
    return 1
  }
  cp "$source_path" "${umber_dir}/document.tex" || {
    case_error "$case_name" "could not stage Umber source"
    return 1
  }
  cp "${source_dir}/support/test2e.tex" "${source_dir}/support/regression-test.tex" \
    "$reference_dir" || {
    case_error "$case_name" "could not stage reference support"
    return 1
  }
  cp "${source_dir}/support/test2e.tex" "${source_dir}/support/regression-test.tex" \
    "$umber_dir" || {
    case_error "$case_name" "could not stage Umber support"
    return 1
  }
  rm -f "${reference_dir}/document.dvi" "${umber_dir}/document.dvi"
  local reference_status=0
  (
    cd "$reference_dir"
    env SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXINPUTS="${local_inputs}:" \
      "$reference_latex" -interaction=batchmode document.tex \
        > document.stdout 2> document.stderr < /dev/null
  ) || reference_status=$?
  if [[ ! -f "${reference_dir}/document.dvi" ]]; then
    printf 'LaTeX DVI parity: %s has no classic LaTeX DVI (status %s)\n' \
      "$case_name" "$reference_status"
    return 2
  fi

  stage_format "$case_name" "$umber_dir"

  local umber_status=0
  (
    cd "$umber_dir"
    env SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXINPUTS="$local_inputs" TEXFONTS="$texfonts" \
      "$umber_bin" run --latex document.tex --format latex.fmt --dvi document.dvi \
        > document.stdout 2> document.stderr < /dev/null
  ) || umber_status=$?
  if [[ ! -f "${umber_dir}/document.dvi" ]]; then
    case_error "$case_name" "Umber emitted no DVI (status $umber_status)"
    return 1
  fi

  if ! "$parity_bin" --compare-existing-dvi \
    "${reference_dir}/document.dvi" "${umber_dir}/document.dvi" \
    --label "$case_name" --triage-dir "$triage_dir"; then
    case_error "$case_name" "coordinate-exact DVI mismatch"
    return 1
  fi
  printf 'LaTeX DVI parity: %s (%s)\n' "$case_name" "$path"
}

[[ -f "$case_list" ]] || fail "setup did not produce case list: $case_list"
selected=0
dvi_selected=0
failed=0
failures="${work_root}/failures.txt"
non_dvi="${work_root}/non-dvi.txt"
: > "$failures"
: > "$non_dvi"
while IFS= read -r path; do
  [[ -n "$path" ]] || continue
  case_name="${path%.lvt}"
  case_name="${case_name//\//--}"
  [[ -z "$case_filter" || "$case_name" == "$case_filter" || "$path" == "$case_filter" ]] || continue
  selected=$((selected + 1))
  status=0
  run_one_case "$path" "$case_name" || status=$?
  if [[ $status -eq 2 ]]; then
    printf '%s\t%s\n' "$case_name" "$path" >> "$non_dvi"
  else
    dvi_selected=$((dvi_selected + 1))
  fi
  if [[ $status -eq 1 ]]; then
    failed=$((failed + 1))
    printf '%s\t%s\n' "$case_name" "$path" >> "$failures"
  fi
done < "$case_list"

[[ $selected -gt 0 ]] || fail "no manifest case matched '${case_filter:-the suite}'"
receipt_cases="$(awk '$1 == "case" { count++ } END { print count + 0 }' "$active_receipt")"
[[ "$receipt_cases" == "$dvi_selected" ]] || fail "format receipt omitted a DVI case"
if [[ $dvi_selected -gt 0 ]]; then
  receipt_hashes="$(awk '$1 == "case" { print $3 }' "$active_receipt" | sort -u | wc -l | tr -d ' ')"
  [[ "$receipt_hashes" == 1 ]] || fail "selected DVI cases did not restore one format identity"
fi
if [[ -z "$case_filter" ]]; then
  expected_dvi_cases="$(awk '$1 == "expected_dvi_cases" { print $2 }' "$manifest")"
  [[ "$dvi_selected" == "$expected_dvi_cases" ]] || \
    fail "derived $dvi_selected classic LaTeX DVI cases; expected $expected_dvi_cases"
fi
printf 'LaTeX format reuse: %s cases restored sha256:%s (builder invocations: %s)\n' \
  "$dvi_selected" "$format_sha256" "$format_build_count"
printf 'LaTeX DVI census: %s candidates, %s classic DVI cases, %s non-DVI configurations (%s)\n' \
  "$selected" "$dvi_selected" "$((selected - dvi_selected))" "$non_dvi"
mv "$active_receipt" "$receipt"
if [[ $failed -gt 0 ]]; then
  printf 'LaTeX DVI parity failures: %s of %s; list: %s\n' "$failed" "$dvi_selected" "$failures" >&2
  exit 1
fi
