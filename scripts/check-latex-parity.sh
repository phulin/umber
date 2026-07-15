#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="${repo_root}/tests/latex-parity-manifest.txt"
source_dir="${repo_root}/third_party/latex2e-parity/source"
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
triage_dir="${target_dir}/latex-parity/triage"

usage() {
  cat <<'EOF'
usage: scripts/check-latex-parity.sh [options]

Options:
  --format PATH       Reuse an existing pregenerated Umber latex.fmt.
  --case NAME         Run one manifest case instead of the complete cohort.
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
  cat > "$receipt" <<EOF
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
  printf 'case %s %s %s\n' "$case_name" "$staged_hash" "$staged" >> "$receipt"
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
  [[ "$(awk '$1 == "case" { print $3 }' "$receipt" | sort -u | wc -l | tr -d ' ')" == 1 ]] || \
    fail "self-test staged more than one format identity"
  [[ "$(awk '$1 == "case" { count++ } END { print count + 0 }' "$receipt")" == 3 ]] || \
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
texinputs=".:${texmf_dist}/tex/latex/base:${texmf_dist}/tex/latex/l3kernel:${texmf_dist}/tex/latex/l3backend:${texmf_dist}/tex/generic/unicode-data:${texmf_dist}/tex/generic/babel:${texmf_dist}/tex/generic/hyphen"
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

selected=0
while read -r kind name path expected_bytes expected_hash passes categories support_path extra; do
  [[ "$kind" == case ]] || continue
  [[ -z "$case_filter" || "$name" == "$case_filter" ]] || continue
  [[ -z "${extra:-}" ]] || fail "invalid manifest case record for $name"
  selected=$((selected + 1))
  case_root="${work_root}/${name}"
  reference_dir="${case_root}/reference"
  umber_dir="${case_root}/umber"
  mkdir -p "$reference_dir" "$umber_dir"
  cp "${source_dir}/${path}" "${reference_dir}/document.tex"
  cp "${source_dir}/${path}" "${umber_dir}/document.tex"
  cp "${source_dir}/${support_path}" "${reference_dir}/$(basename "$support_path")"
  cp "${source_dir}/${support_path}" "${umber_dir}/$(basename "$support_path")"
  stage_format "$name" "$umber_dir"

  for ((pass = 1; pass <= passes; pass++)); do
    rm -f "${reference_dir}/document.dvi" "${umber_dir}/document.dvi"
    reference_status=0
    (
      cd "$reference_dir"
      env SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
        "$reference_latex" -interaction=batchmode document.tex > document.stdout 2> document.stderr
    ) || reference_status=$?
    [[ -f "${reference_dir}/document.dvi" ]] || \
      fail "reference LaTeX emitted no DVI for $name, pass $pass (status $reference_status)"
    umber_status=0
    (
      cd "$umber_dir"
      env SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
        TEXINPUTS="$texinputs" TEXFONTS="$texfonts" \
        "$umber_bin" run --latex document.tex --format latex.fmt --dvi document.dvi \
          > document.stdout 2> document.stderr
    ) || umber_status=$?
    [[ -f "${umber_dir}/document.dvi" ]] || \
      fail "Umber emitted no DVI for $name, pass $pass (status $umber_status)"
  done
  "$parity_bin" --compare-existing-dvi \
    "${reference_dir}/document.dvi" "${umber_dir}/document.dvi" \
    --label "$name" --triage-dir "$triage_dir" || fail "coordinate-exact DVI mismatch for $name"
  printf 'LaTeX DVI parity: %s (%s; %s pass(es))\n' "$name" "$categories" "$passes"
done < "$manifest"

[[ $selected -gt 0 ]] || fail "no manifest case matched '${case_filter:-the suite}'"
receipt_cases="$(awk '$1 == "case" { count++ } END { print count + 0 }' "$receipt")"
[[ "$receipt_cases" == "$selected" ]] || fail "format receipt omitted a selected case"
receipt_hashes="$(awk '$1 == "case" { print $3 }' "$receipt" | sort -u | wc -l | tr -d ' ')"
[[ "$receipt_hashes" == 1 ]] || fail "selected cases did not restore one format identity"
printf 'LaTeX format reuse: %s cases restored sha256:%s (builder invocations: %s)\n' \
  "$selected" "$format_sha256" "$format_build_count"
