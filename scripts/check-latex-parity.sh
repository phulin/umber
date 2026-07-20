#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="${repo_root}/tests/latex-parity-manifest.txt"
source_dir="${repo_root}/third_party/latex2e-parity/source"
case_list="${repo_root}/third_party/latex2e-parity/dvi-cases.txt"
texmf_dist="${UMBER_TEXMF_DIST:-${repo_root}/third_party/texlive-20260301-texmf/texmf-dist}"
reference_latex="${UMBER_REF_LATEX:-$(command -v latex || true)}"
format_builder="${UMBER_LATEX_FORMAT_BUILDER:-${repo_root}/scripts/build-latex-format.sh}"
format_file=""
case_filter=""
offline=0
keep_work=0
self_test=0
format_build_count=0
case_timeout_seconds="${UMBER_LATEX_CASE_TIMEOUT_SECONDS:-60}"

target_dir="${CARGO_TARGET_DIR:-${repo_root}/target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="${repo_root}/${target_dir}"
fi
format_output_dir="${target_dir}/latex-parity/format"
receipt="${target_dir}/latex-parity/last-run-format-receipt.txt"
active_receipt="$receipt"
triage_dir="${target_dir}/latex-parity/triage"
failures_report="${target_dir}/latex-parity/last-run-failures.txt"
non_dvi_report="${target_dir}/latex-parity/last-run-non-dvi.txt"
skipped_report="${target_dir}/latex-parity/last-run-skipped.txt"
scratch_parent="${target_dir}/latex-parity/work"

usage() {
  cat <<'EOF'
usage: scripts/check-latex-parity.sh [options]

Options:
  --format PATH       Reuse an existing pregenerated Umber latex.fmt.
  --case NAME         Run one derived case name or repository-relative path.
  --offline           Do not fetch the pinned upstream LaTeX2e snapshot.
  --keep-work         Preserve all reference and Umber work directories.
  --self-test-format-reuse
                      Test the build-once/stage-identically invariant only.
  --self-test-reference-lookup
                      Test recorder provenance enforcement without TeX Live.

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
    --self-test-reference-lookup) self_test=2; shift ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'check-latex-parity.sh: unknown option: %s\n' "$1" >&2; exit 2 ;;
  esac
done

fail() {
  printf 'check-latex-parity.sh: %s\n' "$*" >&2
  exit 1
}

case_error() {
  local case_name="$1"
  shift
  printf 'LaTeX DVI parity failed: %s: %s\n' "$case_name" "$*" >&2
}

run_with_case_timeout() {
  perl -e '$seconds = shift @ARGV; alarm $seconds; exec @ARGV' \
    "$case_timeout_seconds" "$@"
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

canonical_path() {
  perl -MCwd=abs_path -e '
    $path = abs_path(shift @ARGV);
    defined $path or exit 1;
    print $path;
  ' "$1"
}

path_is_within() {
  local path="$1"
  local root="$2"
  [[ "$path" == "$root" || "$path" == "$root/"* ]]
}

validate_recorder_provenance() {
  local case_name="$1"
  local recorder="$2"
  local reference_root="$3"
  local snapshot_root="$4"
  local distribution_root="$5"
  local generated_root="$6"
  local distribution_config="$7"
  local reference_format="$8"
  local recorded_input resolved_input canonical_input

  while IFS= read -r recorded_input; do
    resolved_input="$recorded_input"
    if [[ "$resolved_input" != /* ]]; then
      resolved_input="${reference_root}/${resolved_input}"
    fi
    if ! canonical_input="$(canonical_path "$resolved_input")"; then
      case_error "$case_name" \
        "reference recorder input no longer exists: $recorded_input"
      return 1
    fi
    if path_is_within "$canonical_input" "$reference_root" || \
      path_is_within "$canonical_input" "$snapshot_root" || \
      path_is_within "$canonical_input" "$distribution_root" || \
      path_is_within "$canonical_input" "$generated_root" || \
      [[ "$canonical_input" == "$distribution_config" || \
         "$canonical_input" == "$reference_format" ]]; then
      continue
    fi
    case_error "$case_name" \
      "reference recorder input escaped pinned roots: $canonical_input"
    return 1
  done < <(sed -n '/^INPUT /s/^INPUT //p' "$recorder")
}

case_skip_reason() {
  local path="$1"
  awk -v path="$path" '
    $1 == "skip" && $2 == path {
      $1 = ""
      $2 = ""
      sub(/^[[:space:]]+/, "")
      print
      exit
    }
  ' "$manifest"
}

case_expected_reference_kind() {
  local path="$1"
  if awk -v path="$path" '$1 == "non_dvi" && $2 == path { found = 1 } END { exit !found }' \
    "$manifest"; then
    printf '%s\n' non-dvi
  else
    printf '%s\n' dvi
  fi
}

validate_case_classifications() {
  local expected_cases expected_dvi_cases expected_non_dvi actual_non_dvi path
  expected_cases="$(awk '$1 == "expected_cases" { print $2 }' "$manifest")"
  expected_dvi_cases="$(awk '$1 == "expected_dvi_cases" { print $2 }' "$manifest")"
  expected_non_dvi=$((expected_cases - expected_dvi_cases))
  actual_non_dvi="$(awk '$1 == "non_dvi" { count++ } END { print count + 0 }' "$manifest")"
  [[ "$actual_non_dvi" == "$expected_non_dvi" ]] || \
    fail "manifest pins $actual_non_dvi non-DVI cases; expected $expected_non_dvi"
  [[ "$(awk '$1 == "non_dvi" { print $2 }' "$manifest" | sort -u | wc -l | tr -d ' ')" == "$actual_non_dvi" ]] || \
    fail "manifest contains duplicate non-DVI classifications"
  while read -r path; do
    [[ -n "$path" ]] || continue
    grep -Fqx "$path" "$case_list" || fail "classified path is not a derived case: $path"
    [[ -z "$(case_skip_reason "$path")" ]] || \
      fail "case cannot be both non-DVI and skipped: $path"
  done < <(awk '$1 == "non_dvi" { print $2 }' "$manifest")
  while read -r path; do
    [[ -n "$path" ]] || continue
    grep -Fqx "$path" "$case_list" || fail "skipped path is not a derived case: $path"
  done < <(awk '$1 == "skip" { print $2 }' "$manifest")
}

reap_abandoned_work_roots() {
  local stale owner
  mkdir -p "$scratch_parent"
  shopt -s nullglob
  for stale in "$scratch_parent"/run.*; do
    owner="$(sed -n '1p' "$stale/.owner-pid" 2>/dev/null || true)"
    if [[ "$owner" =~ ^[1-9][0-9]*$ ]] && kill -0 "$owner" 2>/dev/null; then
      continue
    fi
    rm -rf -- "$stale"
  done
  shopt -u nullglob
}

prepare_format() {
  if [[ -z "$format_file" ]]; then
    mkdir -p "$format_output_dir"
    UMBER_LATEX_FORMAT_WORK_ROOT="$scratch_parent" \
      "$format_builder" --texmf-dist "$texmf_dist" --output-dir "$format_output_dir" \
        > /dev/null
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
  mkdir -p "$scratch_parent"
  temp="$(mktemp -d "${scratch_parent}/self-test.XXXXXX")"
  trap "rm -rf '$temp'" EXIT
  local count_file="${temp}/builder-count"
  local work_root_file="${temp}/builder-work-root"
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
printf '%s\n' "$UMBER_LATEX_FORMAT_WORK_ROOT" > "$UMBER_SELF_TEST_WORK_ROOT_FILE"
mkdir -p "$output_dir"
printf 'one immutable pregenerated format\n' > "${output_dir}/latex.fmt"
EOF
  chmod +x "$stub_builder"
  export UMBER_SELF_TEST_COUNT_FILE="$count_file"
  export UMBER_SELF_TEST_WORK_ROOT_FILE="$work_root_file"
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
  [[ "$(cat "$work_root_file")" == "$scratch_parent" ]] || \
    fail "self-test format builder did not receive the parity scratch root"
  [[ "$format_build_count" == 1 ]] || fail "self-test recorded the wrong build count"
  [[ "$(awk '$1 == "case" { print $3 }' "$active_receipt" | sort -u | wc -l | tr -d ' ')" == 1 ]] || \
    fail "self-test staged more than one format identity"
  [[ "$(awk '$1 == "case" { count++ } END { print count + 0 }' "$active_receipt")" == 3 ]] || \
    fail "self-test did not stage every case"
  scratch_parent="${temp}/work"
  mkdir -p "${scratch_parent}/run.abandoned" "${scratch_parent}/run.live"
  printf '%s\n' 999999999 > "${scratch_parent}/run.abandoned/.owner-pid"
  printf '%s\n' "$$" > "${scratch_parent}/run.live/.owner-pid"
  reap_abandoned_work_roots
  [[ ! -e "${scratch_parent}/run.abandoned" ]] || \
    fail "self-test did not reclaim abandoned parity work"
  [[ -d "${scratch_parent}/run.live" ]] || \
    fail "self-test reclaimed a live parity run"
  printf '%s\n' 'LaTeX parity format-reuse self-test: passed (one build, three identical restores)'
}

run_reference_lookup_self_test() {
  local temp
  mkdir -p "$scratch_parent"
  temp="$(mktemp -d "${scratch_parent}/lookup-self-test.XXXXXX")"
  trap "rm -rf '$temp'" EXIT
  local reference_root="${temp}/reference"
  local snapshot_root="${temp}/snapshot"
  local distribution_root="${temp}/texmf-dist"
  local generated_root="${temp}/generated"
  local outside_root="${temp}/ambient-home"
  local distribution_config="${temp}/texmf.cnf"
  local reference_format="${temp}/latex.fmt"
  mkdir -p "$reference_root" "$snapshot_root" "$distribution_root" \
    "$generated_root" "$outside_root"
  printf '%s\n' local > "${reference_root}/document.tex"
  printf '%s\n' snapshot > "${snapshot_root}/support.tex"
  printf '%s\n' distribution > "${distribution_root}/article.cls"
  printf '%s\n' generated > "${generated_root}/font.tfm"
  printf '%s\n' config > "$distribution_config"
  printf '%s\n' format > "$reference_format"
  printf '%s\n' ambient > "${outside_root}/shadow.sty"

  local allowed="${temp}/allowed.fls"
  cat > "$allowed" <<EOF
INPUT document.tex
INPUT ${snapshot_root}/support.tex
INPUT ${distribution_root}/article.cls
INPUT ${generated_root}/font.tfm
INPUT ${distribution_config}
INPUT ${reference_format}
EOF
  validate_recorder_provenance self-test "$allowed" "$reference_root" \
    "$snapshot_root" "$distribution_root" "$generated_root" \
    "$distribution_config" "$reference_format" || \
    fail "reference lookup self-test rejected a declared input root"

  local escaped="${temp}/escaped.fls"
  printf 'INPUT %s\n' "${outside_root}/shadow.sty" > "$escaped"
  if validate_recorder_provenance self-test "$escaped" "$reference_root" \
    "$snapshot_root" "$distribution_root" "$generated_root" \
    "$distribution_config" "$reference_format" 2> /dev/null; then
    fail "reference lookup self-test accepted an ambient input"
  fi
  ln -s "${outside_root}/shadow.sty" "${reference_root}/shadow-link.sty"
  printf '%s\n' 'INPUT shadow-link.sty' > "$escaped"
  if validate_recorder_provenance self-test "$escaped" "$reference_root" \
    "$snapshot_root" "$distribution_root" "$generated_root" \
    "$distribution_config" "$reference_format" 2> /dev/null; then
    fail "reference lookup self-test accepted a symlink escape"
  fi
  printf '%s\n' 'LaTeX parity reference-lookup self-test: passed'
}

if [[ $self_test -eq 1 ]]; then
  run_format_reuse_self_test
  exit 0
fi
if [[ $self_test -eq 2 ]]; then
  run_reference_lookup_self_test
  exit 0
fi

[[ -f "$manifest" ]] || fail "missing parity manifest: $manifest"
if [[ $offline -eq 1 ]]; then
  "${repo_root}/scripts/setup-latex-parity-tests.sh" --offline > /dev/null
else
  "${repo_root}/scripts/setup-latex-parity-tests.sh" > /dev/null
fi
[[ -x "$reference_latex" ]] || fail "missing reference LaTeX; set UMBER_REF_LATEX"
[[ "$case_timeout_seconds" =~ ^[1-9][0-9]*$ ]] || \
  fail "UMBER_LATEX_CASE_TIMEOUT_SECONDS must be a positive integer"
command -v perl >/dev/null 2>&1 || fail "Perl is required for per-case timeouts"
reference_version="$($reference_latex --version | sed -n '1p')"
[[ "$reference_version" == *'TeX Live 2025'* ]] || \
  fail "reference LaTeX is not from pinned TeX Live 2025: $reference_version"
[[ -d "$texmf_dist" ]] || fail "missing pinned texmf-dist root: $texmf_dist"
texmf_dist="$(canonical_path "$texmf_dist")"
[[ "$(basename "$texmf_dist")" == texmf-dist ]] || \
  fail "pinned distribution must end in texmf-dist: $texmf_dist"
texlive_root="$(dirname "$texmf_dist")"
texlive_config="$(canonical_path "${texlive_root}/texmf.cnf")" || \
  fail "missing pinned TeX Live configuration: ${texlive_root}/texmf.cnf"
texlive_sysvar="${texlive_root}/texmf-var"
reference_format="$(canonical_path "${texlive_sysvar}/web2c/pdftex/latex.fmt")" || \
  fail "missing pinned reference format: ${texlive_sysvar}/web2c/pdftex/latex.fmt"
source_dir="$(canonical_path "$source_dir")"

cd "$repo_root"
prepare_format
start_receipt
cargo build --quiet --release -p umber
cargo build --quiet -p parity-harness --features reference-tools

umber_bin="${target_dir}/release/umber"
parity_bin="${target_dir}/debug/parity-harness"
[[ -x "$umber_bin" && -x "$parity_bin" ]] || fail "required parity binaries were not built"
source_date_epoch="$(awk '$1 == "source_date_epoch" { print $2 }' "$manifest")"
texinput_rel_dirs=(
  tex/latex/base tex/latex/firstaid tex/latex/tools tex/latex/graphics tex/latex/graphics-def
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
texfonts="${texmf_dist}/fonts/tfm/public/cm:${texmf_dist}/fonts/tfm/public/latex-fonts:${texmf_dist}/fonts/tfm/public/amsfonts/cmextra:${texmf_dist}/fonts/tfm/public/amsfonts/euler:${texmf_dist}/fonts/tfm/public/amsfonts/symbols:${texmf_dist}/fonts/tfm/public/amsfonts/cyrillic:${texmf_dist}/fonts/tfm/jknappen/ec"
reap_abandoned_work_roots
work_root="$(mktemp -d "${scratch_parent}/run.XXXXXX")"
printf '%s\n' "$$" > "${work_root}/.owner-pid"
cleanup() {
  if [[ $keep_work -eq 0 ]]; then
    rm -rf "$work_root"
  else
    printf 'LaTeX parity work directory: %s\n' "$work_root" >&2
  fi
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

run_one_case() {
  local path="$1"
  local case_name="$2"
  local skip_umber="$3"
  local expected_reference_kind="$4"
  local case_root="${work_root}/${case_name}"
  local reference_dir="${case_root}/reference"
  local umber_dir="${case_root}/umber"
  local source_path="${source_dir}/${path}"
  local source_parent
  source_parent="$(dirname "$source_path")"
  local local_inputs=".:${source_parent}:${source_dir}/support:${source_dir}/base:${source_dir}/required/tools:${source_dir}/required/graphics:${source_dir}/required/amsmath:${texinputs}"
  local generated_root="${case_root}/reference-state"
  mkdir -p "$reference_dir" "$umber_dir" \
    "${generated_root}/home" "${generated_root}/config" \
    "${generated_root}/var/fonts" "${generated_root}/cache" \
    "${generated_root}/tmp"
  reference_dir="$(canonical_path "$reference_dir")"
  generated_root="$(canonical_path "$generated_root")"
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
    run_with_case_timeout env -i PATH="$PATH" HOME="${generated_root}/home" LC_ALL=C \
      TMPDIR="${generated_root}/tmp" \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${texlive_root}:${texmf_dist}/web2c" \
      TEXMFROOT="$texlive_root" TEXMFDIST="$texmf_dist" \
      TEXMFLOCAL="${generated_root}/local" TEXMFHOME="${generated_root}/home" \
      TEXMFSYSVAR="$texlive_sysvar" TEXMFSYSCONFIG="${generated_root}/sysconfig" \
      TEXMFVAR="${generated_root}/var" TEXMFCONFIG="${generated_root}/config" \
      TEXMFCACHE="${generated_root}/cache" \
      VARTEXFONTS="${generated_root}/var/fonts" \
      TEXFORMATS="${texlive_sysvar}/web2c/pdftex" \
      TEXINPUTS="$local_inputs" TEXFONTS="$texfonts" \
      "$reference_latex" -recorder -interaction=batchmode document.tex \
        > document.stdout 2> document.stderr < /dev/null
  ) || reference_status=$?
  if [[ $reference_status -eq 142 ]]; then
    case_error "$case_name" "reference timed out after ${case_timeout_seconds}s"
    return 1
  fi
  if [[ ! -f "${reference_dir}/document.fls" ]]; then
    case_error "$case_name" "reference recorder did not emit document.fls"
    return 1
  fi
  if ! validate_recorder_provenance "$case_name" \
    "${reference_dir}/document.fls" "$reference_dir" "$source_dir" \
    "$texmf_dist" "$generated_root" "$texlive_config" "$reference_format"; then
    return 1
  fi
  if [[ ! -f "${reference_dir}/document.dvi" ]]; then
    return 2
  fi
  if [[ "$expected_reference_kind" == non-dvi ]]; then
    case_error "$case_name" "reference unexpectedly emitted DVI for pinned non-DVI case"
    return 4
  fi
  if [[ "$skip_umber" -eq 1 ]]; then
    return 3
  fi

  # Mirror every declared external input directory opened by the reference job.
  # The recorder provenance check above rejects ambient user/local trees before
  # any reference-discovered directory reaches Umber.
  # Exclude the reference work directory so generated files such as document.aux
  # cannot leak from the reference run into the isolated Umber run.
  local case_texinputs="$local_inputs"
  local recorded_input resolved_input resolved_input_dir
  while IFS= read -r recorded_input; do
    resolved_input="$recorded_input"
    if [[ "$resolved_input" != /* ]]; then
      resolved_input="${reference_dir}/${resolved_input}"
    fi
    [[ -f "$resolved_input" && "$resolved_input" != *.tfm ]] || continue
    resolved_input_dir="$(cd "${resolved_input%/*}" && pwd -P)"
    [[ "$resolved_input_dir" != "$reference_dir" ]] || continue
    if [[ ":${case_texinputs}:" != *":${resolved_input_dir}:"* ]]; then
      case_texinputs+=":${resolved_input_dir}"
    fi
  done < <(sed -n '/^INPUT /s/^INPUT //p' "${reference_dir}/document.fls")

  # Include every TFM the reference job opened, even when that font never
  # reaches the DVI. Missing one such load shifts TeX's later font numbers and
  # breaks byte-exact output. Leaf directories keep Umber's explicit resolver
  # equivalent without relying on recursive kpathsea path syntax.
  local case_texfonts="$texfonts"
  local recorded_tfm resolved_tfm resolved_dir
  while IFS= read -r recorded_tfm; do
    resolved_tfm="$recorded_tfm"
    if [[ "$resolved_tfm" != /* ]]; then
      resolved_tfm="${reference_dir}/${resolved_tfm}"
    fi
    [[ -f "$resolved_tfm" ]] || continue
    resolved_dir="${resolved_tfm%/*}"
    if [[ ":${case_texfonts}:" != *":${resolved_dir}:"* ]]; then
      case_texfonts+=":${resolved_dir}"
    fi
  done < <(
    sed -n '/^INPUT .*\.tfm$/s/^INPUT //p' "${reference_dir}/document.fls"
  )
  stage_format "$case_name" "$umber_dir"

  local umber_status=0
  (
    cd "$umber_dir"
    run_with_case_timeout env SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXINPUTS="$case_texinputs" TEXFONTS="$case_texfonts" \
      "$umber_bin" run --latex document.tex --format latex.fmt --dvi document.dvi \
        > document.stdout 2> document.stderr < /dev/null
  ) || umber_status=$?
  if [[ ! -f "${umber_dir}/document.dvi" ]]; then
    if [[ $umber_status -eq 142 ]]; then
      case_error "$case_name" "Umber timed out after ${case_timeout_seconds}s"
    else
      case_error "$case_name" "Umber emitted no DVI (status $umber_status)"
    fi
    return 1
  fi

  if ! "$parity_bin" --compare-existing-dvi \
    "${reference_dir}/document.dvi" "${umber_dir}/document.dvi" \
    --label "$case_name" --triage-dir "$triage_dir"; then
    case_error "$case_name" "coordinate-exact DVI mismatch"
    return 1
  fi
}

[[ -f "$case_list" ]] || fail "setup did not produce case list: $case_list"
validate_case_classifications
selected=0
dvi_selected=0
failed=0
skipped_count=0
mkdir -p "$(dirname "$failures_report")"
failures="${failures_report}.$$"
non_dvi="${non_dvi_report}.$$"
skipped="${skipped_report}.$$"
: > "$failures"
: > "$non_dvi"
: > "$skipped"
while IFS= read -r path; do
  [[ -n "$path" ]] || continue
  case_name="${path%.lvt}"
  case_name="${case_name//\//--}"
  [[ -z "$case_filter" || "$case_name" == "$case_filter" || "$path" == "$case_filter" ]] || continue
  selected=$((selected + 1))
  skip_reason="$(case_skip_reason "$path")"
  expected_reference_kind="$(case_expected_reference_kind "$path")"
  skip_umber=0
  [[ -z "$skip_reason" ]] || skip_umber=1
  status=0
  run_one_case "$path" "$case_name" "$skip_umber" "$expected_reference_kind" || status=$?
  if [[ $status -eq 2 ]]; then
    if [[ "$expected_reference_kind" == non-dvi ]]; then
      printf '%s\t%s\n' "$case_name" "$path" >> "$non_dvi"
    else
      case_error "$case_name" "reference emitted no DVI but manifest requires DVI"
      failed=$((failed + 1))
      printf '%s\t%s\n' "$case_name" "$path" >> "$failures"
    fi
  elif [[ $status -eq 3 ]]; then
    skipped_count=$((skipped_count + 1))
    printf '%s\t%s\t%s\n' "$case_name" "$path" "$skip_reason" >> "$skipped"
  elif [[ $status -eq 0 || $status -eq 1 ]]; then
    dvi_selected=$((dvi_selected + 1))
  fi
  if [[ $status -eq 1 || $status -eq 4 ]]; then
    failed=$((failed + 1))
    printf '%s\t%s\n' "$case_name" "$path" >> "$failures"
  fi
  if [[ $keep_work -eq 0 ]]; then
    rm -rf "${work_root:?}/${case_name}"
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
  [[ "$((dvi_selected + skipped_count))" == "$expected_dvi_cases" ]] || \
    fail "derived $dvi_selected tested plus $skipped_count skipped classic LaTeX DVI cases; expected $expected_dvi_cases"
fi
printf 'LaTeX format reuse: %s cases restored sha256:%s (builder invocations: %s)\n' \
  "$dvi_selected" "$format_sha256" "$format_build_count"
mv "$failures" "$failures_report"
mv "$non_dvi" "$non_dvi_report"
mv "$skipped" "$skipped_report"
printf 'LaTeX DVI census: %s candidates, %s tested classic DVI cases, %s skipped unsupported cases (%s), %s non-DVI configurations (%s)\n' \
  "$selected" "$dvi_selected" "$skipped_count" "$skipped_report" \
  "$((selected - dvi_selected - skipped_count))" "$non_dvi_report"
mv "$active_receipt" "$receipt"
if [[ $failed -gt 0 ]]; then
  printf 'LaTeX DVI parity: %s exact, %s failures of %s; list: %s\n' \
    "$((dvi_selected - failed))" "$failed" "$dvi_selected" "$failures_report" >&2
  exit 1
fi
printf 'LaTeX DVI parity: %s exact, 0 failures of %s\n' \
  "$dvi_selected" "$dvi_selected"
