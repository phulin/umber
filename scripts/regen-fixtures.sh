#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

text_areas=(hello lexer expand lexer_dynamic exec typeset tex_exec tex_exec_io)
dvi_areas=(dvi page math align leaders)

target_dir="${CARGO_TARGET_DIR:-target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="${repo_root}/${target_dir}"
fi
refexec_bin="${target_dir}/debug/refexec"
fixturegen_bin="${target_dir}/debug/fixturegen"
umber_bin="${target_dir}/debug/umber"
refexec_built=0
fixturegen_built=0
umber_built=0
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1783604160}"

usage() {
  cat <<'EOF'
usage:
  scripts/regen-fixtures.sh --incremental
  scripts/regen-fixtures.sh --all
  scripts/regen-fixtures.sh --area AREA
  scripts/regen-fixtures.sh --case AREA/CASE
  scripts/regen-fixtures.sh --case AREA CASE

Fixture areas:
  text/native: hello lexer expand lexer_dynamic exec typeset tex_exec tex_exec_io
  DVI:         dvi page math align leaders
  live check:  fonts  (runs the tftopl cross-check; it does not rewrite fixtures)

Reference tools:
  Text and DVI reference regeneration requires pdftex or tex on PATH, or
  UMBER_REF_TEX=/absolute/path/to/pdftex. Text/native regeneration builds and
  runs the workspace fixturegen tool; DVI regeneration builds and runs the
  workspace refexec tool and copies pinned CM TFMs from
  crates/tex-fonts/tests/fixtures/cm plus area-local support files.

  The fonts live check requires tftopl on PATH, or
  UMBER_REF_TFTOPL=/absolute/path/to/tftopl.

  Regeneration pins SOURCE_DATE_EPOCH by default so Umber and the reference
  TeX observe the same job-start clock. Set SOURCE_DATE_EPOCH explicitly to
  override the default fixed timestamp.
EOF
}

die() {
  printf 'regen-fixtures: %s\n' "$*" >&2
  exit 2
}

contains_area() {
  local needle="$1"
  shift
  local area
  for area in "$@"; do
    if [[ "$area" == "$needle" ]]; then
      return 0
    fi
  done
  return 1
}

is_text_area() {
  contains_area "$1" "${text_areas[@]}"
}

is_dvi_area() {
  contains_area "$1" "${dvi_areas[@]}"
}

is_known_area() {
  is_text_area "$1" || is_dvi_area "$1" || [[ "$1" == "fonts" ]]
}

test_command_for_area() {
  local area="$1"
  case "$area" in
    hello)
      printf '%s\n' 'cargo test -p test-support hello_fixture_is_committed'
      ;;
    lexer)
      printf '%s\n' 'cargo test -p umber --test it lex_dump_prints_stable_token_format_for_corpus'
      ;;
    expand)
      printf '%s\n' 'cargo test -p umber --test it expand_dump_prints_stable_token_format_for_corpus'
      ;;
    lexer_dynamic)
      printf '%s\n' 'cargo test -p umber --test it lexer_dynamic_corpus_covers_mutable_input_state'
      ;;
    exec)
      printf '%s\n' 'cargo test -p umber --test it run_exec_corpus_matches_committed_diagnostics'
      ;;
    typeset)
      printf '%s\n' 'cargo test -p umber --test it run_typeset_corpus_matches_committed_box_dumps'
      ;;
    tex_exec)
      printf '%s\n' 'cargo test -p tex-exec --lib grouping_parity'
      ;;
    tex_exec_io)
      printf '%s\n' 'cargo test -p tex-exec --lib io::'
      ;;
    dvi)
      printf '%s\n' 'cargo test -p umber --test it run_dvi_corpus_matches_committed_dvi'
      ;;
    page)
      printf '%s\n' 'cargo test -p umber --test it run_page_corpus_matches_committed_dvi'
      ;;
    math)
      printf '%s\n' 'cargo test -p umber --test it run_math_corpus_matches_committed_dvi'
      ;;
    align)
      printf '%s\n' 'cargo test -p umber --test it run_align_corpus_matches_committed_dvi'
      ;;
    leaders)
      printf '%s\n' 'cargo test -p umber --test it run_leaders_corpus_matches_committed_dvi'
      ;;
    *)
      die "unknown fixture area: ${area}"
      ;;
  esac
}

run_command() {
  local description="$1"
  shift
  printf '%s\n' "$description" >&2
  "$@"
}

build_refexec_once() {
  if [[ "$refexec_built" -eq 0 ]]; then
    run_command 'Building refexec' cargo build -p refexec
    refexec_built=1
  fi
}

build_fixturegen_once() {
  if [[ "$fixturegen_built" -eq 0 ]]; then
    run_command 'Building fixturegen' cargo build -p fixturegen
    fixturegen_built=1
  fi
}

build_umber_once() {
  if [[ "$umber_built" -eq 0 ]]; then
    run_command 'Building umber' cargo build -p umber
    umber_built=1
  fi
}

fixturegen_needs_umber() {
  case "$1" in
    lexer|expand)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

copy_dvi_inputs() {
  local area="$1"
  local case_name="$2"
  local case_dir="$3"
  local corpus_dir="${repo_root}/tests/corpus/${area}"
  local cm_tfm_dir="${repo_root}/crates/tex-fonts/tests/fixtures/cm"
  local tfm
  local support
  local support_name

  cp "${corpus_dir}/${case_name}" "${case_dir}/${case_name}"
  for tfm in cmr10.tfm cmmi10.tfm cmsy10.tfm cmex10.tfm; do
    cp "${cm_tfm_dir}/${tfm}" "${case_dir}/${tfm}"
    dvi_extra_inputs+=(--extra-input "$tfm")
  done
  while IFS= read -r support; do
    support_name="$(basename "$support")"
    cp "$support" "${case_dir}/${support_name}"
    dvi_extra_inputs+=(--extra-input "$support_name")
  done < <(find "$corpus_dir" -maxdepth 1 -type f ! -name '*.tex' ! -name '*.expected.*' | sort)
}

regen_dvi_case() {
  local area="$1"
  local case="$2"
  local corpus_dir="${repo_root}/tests/corpus/${area}"
  local source="${corpus_dir}/${case}.tex"
  local fixture="${corpus_dir}/${case}.expected.dvi"
  local case_name="${case}.tex"
  local tmp_root
  local case_dir
  local ref_dvi
  local compare_status
  local ini_arg=""

  [[ -f "$source" ]] || die "missing DVI source: tests/corpus/${area}/${case}.tex"

  build_refexec_once
  tmp_root="$(mktemp -d)"
  case_dir="${tmp_root}/${area}-${case}"
  mkdir -p "$case_dir"
  dvi_extra_inputs=()
  copy_dvi_inputs "$area" "$case_name" "$case_dir"
  if [[ "$area" == "math" ]]; then
    ini_arg="--ini"
  fi

  printf 'Regenerating DVI fixture %s/%s\n' "$area" "$case" >&2
  if [[ -n "$ini_arg" ]]; then
    (
      cd "$case_dir"
      "$refexec_bin" "$case_name" --dvi "$ini_arg" "${dvi_extra_inputs[@]}"
    )
  else
    (
      cd "$case_dir"
      "$refexec_bin" "$case_name" --dvi "${dvi_extra_inputs[@]}"
    )
  fi
  ref_dvi="${case_dir}/${case}.ref.dvi"
  [[ -f "$ref_dvi" ]] || die "reference DVI was not written for ${area}/${case}"

  if [[ -f "$fixture" ]]; then
    set +e
    if [[ -n "$ini_arg" ]]; then
      (
        cd "$case_dir"
        "$refexec_bin" "$case_name" --compare-dvi "$fixture" \
          "$ini_arg" "${dvi_extra_inputs[@]}"
      )
    else
      (
        cd "$case_dir"
        "$refexec_bin" "$case_name" --compare-dvi "$fixture" \
          "${dvi_extra_inputs[@]}"
      )
    fi
    compare_status=$?
    set -e
    if [[ "$compare_status" -eq 0 ]]; then
      printf 'DVI fixture unchanged: %s/%s\n' "$area" "$case" >&2
      rm -rf "$tmp_root"
      return
    fi
  fi

  cp "$ref_dvi" "$fixture"
  printf 'DVI fixture updated: %s\n' "${fixture#"$repo_root"/}" >&2
  rm -rf "$tmp_root"
}

validate_dvi_area() {
  local area="$1"
  local command_string
  command_string="$(test_command_for_area "$area")"
  # shellcheck disable=SC2086
  run_command "Validating ${area} DVI fixtures" $command_string
}

regen_text_area() {
  local area="$1"
  local command_string
  build_fixturegen_once
  if fixturegen_needs_umber "$area"; then
    build_umber_once
  fi
  run_command "Regenerating ${area} fixtures" \
    env UMBER_BIN="$umber_bin" "$fixturegen_bin" --area "$area"
  command_string="$(test_command_for_area "$area")"
  # shellcheck disable=SC2086
  run_command "Validating ${area} fixtures" $command_string
}

regen_dvi_area() {
  local area="$1"
  local source
  local case
  local corpus_dir="${repo_root}/tests/corpus/${area}"
  for source in "${corpus_dir}"/*.tex; do
    [[ -e "$source" ]] || continue
    case="$(basename "$source" .tex)"
    regen_dvi_case "$area" "$case"
  done
  validate_dvi_area "$area"
}

run_fonts_live_check() {
  build_fixturegen_once
  run_command 'Running live tftopl font cross-check' "$fixturegen_bin" --area fonts
}

regen_area() {
  local area="$1"
  is_known_area "$area" || die "unknown fixture area: ${area}"
  if is_text_area "$area"; then
    regen_text_area "$area"
  elif is_dvi_area "$area"; then
    regen_dvi_area "$area"
  else
    run_fonts_live_check
  fi
}

case_area_from_arg() {
  local spec="$1"
  printf '%s\n' "${spec%%/*}"
}

case_name_from_arg() {
  local spec="$1"
  local name="${spec#*/}"
  name="${name%.tex}"
  name="${name%.expected.dvi}"
  name="${name%.expected.log}"
  name="${name%.expected.tokens}"
  name="${name%.expected.ref}"
  name="${name%.expected.out}"
  name="${name%.expected.effects}"
  name="${name%.expected.specials}"
  printf '%s\n' "$name"
}

regen_case() {
  local area="$1"
  local case="$2"
  local command_string
  is_known_area "$area" || die "unknown fixture area: ${area}"
  if is_dvi_area "$area"; then
    regen_dvi_case "$area" "$case"
    validate_dvi_area "$area"
  elif is_text_area "$area"; then
    printf 'Regenerating text area %s for requested case %s\n' "$area" "$case" >&2
    build_fixturegen_once
    if fixturegen_needs_umber "$area"; then
      build_umber_once
    fi
    run_command "Regenerating ${area}/${case} fixture" \
      env UMBER_BIN="$umber_bin" "$fixturegen_bin" --case "$area" "$case"
    command_string="$(test_command_for_area "$area")"
    # shellcheck disable=SC2086
    run_command "Validating ${area} fixtures" $command_string
  else
    die "--case is not meaningful for the fonts live check"
  fi
}

incremental_specs() {
  {
    git diff --name-only -- tests/corpus
    git diff --name-only --cached -- tests/corpus
    git ls-files --others --exclude-standard -- tests/corpus
  } | sort -u
}

regen_incremental() {
  local path
  local area
  local file
  local stem
  local case_specs_file
  local area_specs_file
  local spec

  case_specs_file="$(mktemp)"
  area_specs_file="$(mktemp)"

  while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    case "$path" in
      tests/corpus/*/*)
        area="${path#tests/corpus/}"
        area="${area%%/*}"
        file="$(basename "$path")"
        if ! is_known_area "$area"; then
          continue
        fi
        case "$file" in
          *.tex)
            stem="${file%.tex}"
            printf '%s/%s\n' "$area" "$stem" >>"$case_specs_file"
            ;;
          *.expected.*)
            stem="${file%%.expected.*}"
            printf '%s/%s\n' "$area" "$stem" >>"$case_specs_file"
            ;;
          *)
            printf '%s\n' "$area" >>"$area_specs_file"
            ;;
        esac
        ;;
    esac
  done < <(incremental_specs)

  if [[ ! -s "$case_specs_file" && ! -s "$area_specs_file" ]]; then
    printf 'No changed fixture sources or expected files found under tests/corpus.\n' >&2
    rm -f "$case_specs_file" "$area_specs_file"
    return
  fi

  while IFS= read -r spec; do
    [[ -n "$spec" ]] || continue
    regen_case "$(case_area_from_arg "$spec")" "$(case_name_from_arg "$spec")"
  done < <(sort -u "$case_specs_file")

  while IFS= read -r area; do
    [[ -n "$area" ]] || continue
    regen_area "$area"
  done < <(sort -u "$area_specs_file")

  rm -f "$case_specs_file" "$area_specs_file"
}

mode=""
area_arg=""
case_area=""
case_name=""

if [[ "$#" -eq 0 ]]; then
  usage
  exit 2
fi

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --incremental)
      [[ -z "$mode" ]] || die "choose exactly one mode"
      mode="incremental"
      shift
      ;;
    --all)
      [[ -z "$mode" ]] || die "choose exactly one mode"
      mode="all"
      shift
      ;;
    --area)
      [[ -z "$mode" ]] || die "choose exactly one mode"
      [[ "$#" -ge 2 ]] || die "missing area after --area"
      mode="area"
      area_arg="$2"
      shift 2
      ;;
    --case)
      [[ -z "$mode" ]] || die "choose exactly one mode"
      [[ "$#" -ge 2 ]] || die "missing case after --case"
      mode="case"
      if [[ "$2" == */* ]]; then
        case_area="$(case_area_from_arg "$2")"
        case_name="$(case_name_from_arg "$2")"
        shift 2
      else
        [[ "$#" -ge 3 ]] || die "--case requires AREA CASE or AREA/CASE"
        case_area="$2"
        case_name="$(case_name_from_arg "$3")"
        shift 3
      fi
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

case "$mode" in
  incremental)
    regen_incremental
    ;;
  all)
    for area_arg in "${text_areas[@]}"; do
      regen_area "$area_arg"
    done
    for area_arg in "${dvi_areas[@]}"; do
      regen_area "$area_arg"
    done
    ;;
  area)
    regen_area "$area_arg"
    ;;
  case)
    [[ -n "$case_area" && -n "$case_name" ]] || die "missing case"
    regen_case "$case_area" "$case_name"
    ;;
  *)
    usage
    exit 2
    ;;
esac
