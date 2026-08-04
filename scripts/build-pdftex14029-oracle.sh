#!/usr/bin/env bash
# Canonical TeX Live 2026 pdfTeX 1.40.29 oracle builder.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

manifest="tests/pdftex14029-oracle-manifest.txt"
source_lock="tests/texlive-source.lock"
source_name="$(awk '$1 == "archive" { print $2 }' "$source_lock")"
cache_root="${repo_root}/third_party/texlive-source"
source_tar="${cache_root}/${source_name}"
source_dir="${cache_root}/src"
build_dir="${cache_root}/build-pdftex14029-20260301"
web_source_dir="${source_dir}/texk/web2c"
web_build_dir="${build_dir}/texk/web2c"
target_dir="${CARGO_TARGET_DIR:-target}"
[[ "$target_dir" == /* ]] || target_dir="${repo_root}/${target_dir}"
out_dir="${target_dir}/pdftex14029-oracle"
bin_dir="${out_dir}/bin"
instrumentation_change="${UMBER_PDFTEX14029_INSTRUMENTATION_CHANGE:-${repo_root}/tests/pdftex14029-oracle/instrumentation.ch}"
extension_instrumentation_change="${UMBER_PDFTEX14029_EXTENSION_INSTRUMENTATION_CHANGE:-${repo_root}/tests/pdftex14029-oracle/extension-instrumentation.ch}"
state_instrumentation_change="${UMBER_PDFTEX14029_STATE_INSTRUMENTATION_CHANGE:-${repo_root}/tests/pdftex14029-oracle/state-instrumentation.ch}"
dvi_input="${repo_root}/tests/pdftex14029-oracle/smoke-dvi.tex"
pdf_input="${repo_root}/tests/pdftex14029-oracle/smoke-pdf.tex"
transition_input="${repo_root}/tests/pdftex14029-oracle/transitions.tex"
transition_child="${repo_root}/tests/pdftex14029-oracle/transitions-child.tex"
case_shift_input="${repo_root}/tests/pdftex14029-oracle/case-shift.tex"
semantic_event_matrix="${repo_root}/tests/pdftex14029-oracle/semantic-event-matrix.txt"
extension_input="${repo_root}/tests/pdftex14029-oracle/extensions.tex"
extension_bytes_input="${repo_root}/tests/pdftex14029-oracle/extensions-bytes.txt"
extension_event_matrix="${repo_root}/tests/pdftex14029-oracle/extension-event-matrix.txt"
etex_profile_input="${repo_root}/tests/pdftex14029-oracle/etex-profile-boundaries.tex"
etex_profile_compatibility_input="${repo_root}/tests/pdftex14029-oracle/etex-profile-compatibility.tex"
etex_profile_recovery_input="${repo_root}/tests/pdftex14029-oracle/etex-profile-recovery.tex"
etex_profile_hyph_format_input="${repo_root}/tests/pdftex14029-oracle/etex-profile-hyph-format.tex"
etex_profile_hyph_input="${repo_root}/tests/pdftex14029-oracle/etex-profile-hyph.tex"
etex_profile_matrix="${repo_root}/tests/pdftex14029-oracle/etex-profile-boundary-matrix.txt"
state_input="${repo_root}/tests/pdftex14029-oracle/state.tex"
state_event_matrix="${repo_root}/tests/pdftex14029-oracle/state-event-matrix.txt"
state_font_input="${web_source_dir}/tests/cmr10.tfm"
state_map_input="${repo_root}/tests/pdftex14029-oracle/pdftex.map"
extension_primitive_audit="${repo_root}/tests/pdftex14029-oracle/extension-primitive-audit.txt"
pdf_normalizer="${target_dir}/debug/pdf-normalize"
wide_tangle="${out_dir}/tangle-pdftex14029"
cflags="-O2"
cxxflags="-O2"
source_date_epoch="${SOURCE_DATE_EPOCH:-1783604160}"
offline=0

usage() {
  cat <<'EOF'
usage: scripts/build-pdftex14029-oracle.sh [--offline]

Acquire and verify the pinned TeX Live 2026 source snapshot, then build
canonical pdfTeX 1.40.29 clean and instrumented eight-bit Web2C executables.
The repository-owned final change is applied only to the latter and emits the
shared schema-v1 command trace.

Outputs and a complete identity record are written under
target/pdftex14029-oracle. After the first acquisition, --offline performs no
network I/O.
EOF
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --offline) offline=1 ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'build-pdftex14029-oracle: unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

fail() {
  printf 'build-pdftex14029-oracle: %s\n' "$*" >&2
  exit 1
}

sha_digest() {
  local bits="$1" path="$2"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a "$bits" "$path" | awk '{print $1}'
  elif command -v "sha${bits}sum" >/dev/null 2>&1; then
    "sha${bits}sum" "$path" | awk '{print $1}'
  else
    fail "need shasum or sha${bits}sum on PATH"
  fi
}

archive_url="$(awk '$1 == "archive" { print $5 }' "$source_lock")"
archive_sha512="$(awk '$1 == "archive" { print $4 }' "$source_lock")"
[[ -n "$source_name" && -n "$archive_url" && -n "$archive_sha512" ]] || fail "invalid $source_lock"

fetch_source() {
  local arguments=(source "$repo_root")
  [[ "$offline" -eq 0 ]] || arguments+=(--offline)
  python3 scripts/provision.py "${arguments[@]}" >/dev/null
}

verify_inputs() {
  [[ -f "$instrumentation_change" ]] ||
    fail "missing instrumentation change file: $instrumentation_change"
  [[ -f "$extension_instrumentation_change" ]] ||
    fail "missing extension instrumentation change file: $extension_instrumentation_change"
}

extract_source() {
  verify_inputs
}

configure_tools() {
  mkdir -p "$build_dir"
  if [[ ! -f "${build_dir}/Makefile" ]]; then
    (
      cd "$build_dir"
      ../src/configure --without-x --disable-shared --disable-all-pkgs \
        --enable-pdftex --disable-synctex --disable-xetex --enable-missing -C \
        CFLAGS="$cflags" CXXFLAGS="$cxxflags"
    )
  fi
  if [[ ! -f "${web_build_dir}/Makefile" ]]; then
    make -C "$build_dir"
  fi
  make -C "$web_build_dir" tie tangle web2c/web2c
}

build_wide_tangle() {
  local generated="${out_dir}/tangle-pdftex14029.c"
  local source="${web_build_dir}/tangle.c"
  local replacements stack_replacements byte_replacements
  mkdir -p "$out_dir"
  replacements="$(grep -c '^#define maxtoks ( 65535L )' "$source")"
  [[ "$replacements" -eq 1 ]] ||
    fail "canonical generated tangle.c has an unexpected token-capacity declaration"
  stack_replacements="$(grep -c '^#define stacksize ( 100 )' "$source")"
  [[ "$stack_replacements" -eq 1 ]] ||
    fail "canonical generated tangle.c has an unexpected stack-capacity declaration"
  byte_replacements="$(grep -c '^#define maxbytes ( 65535L )' "$source")"
  [[ "$byte_replacements" -eq 1 ]] ||
    fail "canonical generated tangle.c has an unexpected name-byte capacity declaration"
  sed \
    -e 's/^#define maxbytes ( 65535L )/#define maxbytes ( 131071L )/' \
    -e 's/^#define maxtoks ( 65535L )/#define maxtoks ( 131071L )/' \
    -e 's/^#define maxnames ( 10239 )/#define maxnames ( 20479 )/' \
    -e 's/^#define maxtexts ( 10239 )/#define maxtexts ( 20479 )/' \
    -e 's/^#define stacksize ( 100 )/#define stacksize ( 5000 )/' \
    -e 's/sixteenbits endfield/integer endfield/' \
    -e 's/sixteenbits bytefield/integer bytefield/' \
    -e 's/^sixteenbits bytestart/integer bytestart/' \
    -e 's/^sixteenbits tokstart/integer tokstart/' \
    "$source" >"$generated"
  cc -DHAVE_CONFIG_H \
    -I"$web_build_dir" -I"$web_source_dir" -I"${web_build_dir}/w2c" \
    -I"${build_dir}/texk" -I"${source_dir}/texk" "$cflags" \
    "$generated" "${web_build_dir}/lib/lib.a" \
    "${build_dir}/texk/kpathsea/.libs/libkpathsea.a" -o "$wide_tangle"
}

web_source="${web_source_dir}/pdftexdir/pdftex.web"
tex_web_source="${web_source_dir}/tex.web"
etex_change_source="${web_source_dir}/etexdir/etex.ch"
upstream_changes=(
  "${web_source_dir}/pdftexdir/tex.ch0"
  "${web_source_dir}/tex.ch"
  "${web_source_dir}/tracingstacklevels.ch"
  "${web_source_dir}/partoken-102.ch"
  "${web_source_dir}/partoken.ch"
  "${web_source_dir}/locnull-optimize.ch"
  "${web_source_dir}/showstream.ch"
  "${web_source_dir}/zlib-fmt.ch"
  "${web_source_dir}/enctexdir/enctex1.ch"
  "${web_source_dir}/enctexdir/enctex-pdftex.ch"
  "${web_source_dir}/enctexdir/enctex2.ch"
  "${web_source_dir}/unbalanced-braces.ch"
  "${web_source_dir}/synctexdir/synctex-def.ch0"
  "${web_source_dir}/synctexdir/synctex-mem.ch0"
  "${web_source_dir}/synctexdir/synctex-e-mem.ch0"
  "${web_source_dir}/synctexdir/synctex-e-mem.ch1"
  "${web_source_dir}/synctexdir/synctex-rec.ch0"
  "${web_source_dir}/synctexdir/synctex-rec.ch1"
  "${web_source_dir}/synctexdir/synctex-e-rec.ch0"
  "${web_source_dir}/synctexdir/synctex-pdf-rec.ch2"
  "${web_source_dir}/pdftexdir/pdftex.ch"
  "${web_source_dir}/pdftexdir/char-warning-pdftex.ch"
  "${web_source_dir}/tex-binpool.ch"
)

reset_pdftex_products() {
  rm -f \
    "${web_build_dir}/pdftex-final.ch" \
    "${web_build_dir}/pdftex-tangle" \
    "${web_build_dir}/pdftex.p" \
    "${web_build_dir}/pdftex.pool" \
    "${web_build_dir}/pdftex-web2c" \
    "${web_build_dir}/pdftexini.c" \
    "${web_build_dir}/pdftex0.c" \
    "${web_build_dir}/pdftexcoerce.h" \
    "${web_build_dir}/pdftexd.h" \
    "${web_build_dir}/pdftex-pool.c" \
    "${web_build_dir}/pdftex-pdftexini.o" \
    "${web_build_dir}/pdftex-pdftex0.o" \
    "${web_build_dir}/pdftex-pdftex-pool.o" \
    "${web_build_dir}/pdftexdir/pdftex-pdftexextra.o" \
    "${web_build_dir}/pdftex"
}

build_variant() {
  local destination="$1"
  shift
  local change_stack=("${upstream_changes[@]}" "$@")
  reset_pdftex_products
  (
    cd "$web_build_dir"
    TEXMFCNF="${source_dir}/texk/kpathsea" WEBINPUTS=".:${web_source_dir}:${web_source_dir}/pdftexdir" \
      ./tie -c pdftex-final.ch "$web_source" "${change_stack[@]}"
    touch pdftex.ch pdftex-tangle
    TEXMFCNF="${source_dir}/texk/kpathsea" WEBINPUTS=".:${web_source_dir}:${web_source_dir}/pdftexdir" \
      "$wide_tangle" pdftex pdftex-final
    AM_V_P=false ./web2c-sh pdftex-web2c pdftex
  )
  make -C "$web_build_dir" pdftex
  [[ -x "${web_build_dir}/pdftex" ]] || fail "pdfTeX executable was not built"
  cp "${web_build_dir}/pdftex" "$destination"
  chmod +x "$destination"
}

profile_executable() {
  local profile="$1"
  printf '%s/umber-pdftex14029-oracle-%s' "$bin_dir" "$profile"
}

normalize_pdf() {
  local input="$1" output="$2"
  cargo run -q -p test-support --bin pdf-normalize -- "$input" >"$output"
}

run_smoke() {
  local executable="$1" profile="$2" mode="$3" input output marker status=0
  local run_dir="${out_dir}/smoke/${profile}-${mode}"
  input="$dvi_input"
  output="smoke-dvi.dvi"
  marker="UMBER-PDFTEX14029-ORACLE-DVI-SMOKE"
  [[ "$mode" == dvi ]] || {
    input="$pdf_input"
    output="smoke-pdf.pdf"
    marker="UMBER-PDFTEX14029-ORACLE-PDF-SMOKE"
  }
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$input" "${run_dir}/$(basename "$input")"
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/kpathsea" \
      "$executable" -ini -etex -interaction=batchmode "$(basename "$input")" \
      >terminal.txt 2>&1
  ) || status="$?"
  [[ "$status" -eq 0 ]] || fail "$profile/$mode smoke exited with status $status"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  [[ -f "${run_dir}/${output}" ]] || fail "$profile/$mode smoke did not write $output"
  grep -q 'pdfTeX, Version 3.141592653-2.6-1.40.29' "${run_dir}/smoke-${mode}.log" ||
    fail "$profile/$mode did not identify canonical pdfTeX 1.40.29"
  grep -q "$marker" "${run_dir}/smoke-${mode}.log" ||
    fail "$profile/$mode smoke marker is absent"
  grep -q 'PDFTEX=14029' "${run_dir}/smoke-${mode}.log" ||
    fail "$profile/$mode pdfTeX version primitive marker is absent"
  grep -q 'ETEX=2.6' "${run_dir}/smoke-${mode}.log" ||
    fail "$profile/$mode e-TeX version primitive marker is absent"
  grep -q '=42' "${run_dir}/smoke-${mode}.log" ||
    fail "$profile/$mode arithmetic marker is absent"
  sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/smoke-${mode}.log" \
    >"${run_dir}/ordinary.log"
  if [[ "$mode" == pdf ]]; then
    normalize_pdf "${run_dir}/${output}" "${run_dir}/normalized-pdf.txt"
  fi
}

compare_smoke_channels() {
  local mode="$1" output="smoke-dvi.dvi"
  [[ "$mode" == dvi ]] || output="smoke-pdf.pdf"
  local left="${out_dir}/smoke/clean-${mode}"
  local right="${out_dir}/smoke/instrumented-${mode}"
  local channel
  for channel in terminal.txt ordinary.log status.txt "$output"; do
    cmp "${left}/${channel}" "${right}/${channel}" >/dev/null ||
      fail "instrumented $mode oracle changed $channel"
  done
}

compare_channels() {
  local label="$1" left="$2" right="$3"
  shift 3
  local channel
  for channel in "$@"; do
    cmp "${left}/${channel}" "${right}/${channel}" >/dev/null ||
      fail "$label changed $channel"
  done
}

run_transitions() {
  local executable="$1" profile="$2"
  local run_dir="${out_dir}/transitions/${profile}" status=0
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$transition_input" "${run_dir}/transitions.tex"
  cp "$transition_child" "${run_dir}/transitions-child.tex"
  cp "$case_shift_input" "${run_dir}/case-shift.tex"
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/kpathsea" \
      "$executable" -ini -etex -interaction=nonstopmode transitions.tex \
      >terminal.txt 2>&1
  ) || status="$?"
  [[ "$status" -le 1 ]] ||
    fail "$profile transition run exited with unexpected status $status"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  [[ -f "${run_dir}/transitions.log" ]] ||
    fail "$profile transition run did not write transitions.log"
  [[ -f "${run_dir}/transitions.dvi" ]] ||
    fail "$profile transition run did not write transitions.dvi"
  [[ -f "${run_dir}/transitions-effects.out" ]] ||
    fail "$profile transition run did not write transition effects"
  grep -q 'UMBER-TEX82-TRANSITIONS' "${run_dir}/transitions.log" ||
    fail "$profile transition marker is absent"
  sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/transitions.log" \
    >"${run_dir}/ordinary.log"
}

run_extensions() {
  local executable="$1" profile="$2"
  local run_dir="${out_dir}/extensions/${profile}" status=0
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$extension_input" "${run_dir}/extensions.tex"
  cp "$extension_bytes_input" "${run_dir}/extensions-bytes.txt"
  TZ=UTC touch -t 202001020304.05 "${run_dir}/extensions-bytes.txt"
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C TZ=UTC \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/kpathsea" \
      "$executable" -ini -etex -interaction=batchmode extensions.tex \
      >terminal.txt 2>&1
  ) || status="$?"
  [[ "$status" -eq 0 ]] ||
    fail "$profile extension run exited with status $status"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  [[ -f "${run_dir}/extensions.log" ]] ||
    fail "$profile extension run did not write extensions.log"
  [[ -f "${run_dir}/extensions.dvi" ]] ||
    fail "$profile extension run did not write extensions.dvi"
  [[ -f "${run_dir}/extensions-effects.out" ]] ||
    fail "$profile extension run did not write extension effects"
  grep -q 'UMBER-PDFTEX14029-COMMAND-EXTENSIONS' \
    "${run_dir}/extensions.log" ||
    fail "$profile extension marker is absent"
  grep -q 'EXPANDED=AEXPANDEDB' "${run_dir}/extensions.log" ||
    fail "$profile expanded result is absent"
  grep -q 'STRCMP=-1,0,1' "${run_dir}/extensions.log" ||
    fail "$profile bytewise comparison result is absent"
  grep -q 'PRIMITIVE-REDEFINED-FALSE' "${run_dir}/extensions.log" ||
    fail "$profile primitive-enquiry result is absent"
  grep -q 'ABSNUM-TRUE' "${run_dir}/extensions.log" &&
    grep -q 'ABSDIM-TRUE' "${run_dir}/extensions.log" ||
    fail "$profile absolute-comparison results are absent"
  grep -q 'IDENTITY=140,29' "${run_dir}/extensions.log" ||
    fail "$profile exact pdfTeX profile identity is absent"
  grep -q 'Version 3.141592653-2.6-1.40.29' "${run_dir}/extensions.log" ||
    fail "$profile canonical pdfTeX banner identity is absent"
  sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/extensions.log" \
    >"${run_dir}/ordinary.log"
}

run_etex_profile_boundaries() {
  local executable="$1" profile="$2" status=0 input marker
  local run_dir="${out_dir}/etex-profile/${profile}"
  input="$etex_profile_input"
  marker=UMBER-PDFTEX14029-ETEX-BOUNDARIES
  if [[ "$profile" == *-compatibility ]]; then
    input="$etex_profile_compatibility_input"
    marker=UMBER-PDFTEX14029-ETEX-COMPATIBILITY-ABSENT
  fi
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$input" "${run_dir}/$(basename "$input")"
  (
    cd "$run_dir"
    if [[ "$profile" == *-compatibility ]]; then
      env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
        SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
        TEXMFCNF="${source_dir}/texk/kpathsea" \
        "$executable" -ini -interaction=batchmode "$(basename "$input")" \
        >terminal.txt 2>&1
    else
      env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
        SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
        TEXMFCNF="${source_dir}/texk/kpathsea" \
        "$executable" -ini -etex -interaction=batchmode "$(basename "$input")" \
        >terminal.txt 2>&1
    fi
  ) || status="$?"
  [[ "$status" -eq 0 ]] || fail "$profile e-TeX boundary run exited with status $status"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  local stem
  stem="$(basename "$input" .tex)"
  grep -q "$marker" "${run_dir}/${stem}.log" ||
    fail "$profile e-TeX boundary marker is absent"
  if [[ "$profile" == *-extended ]]; then
    grep -q 'ROLLBACK-OK' "${run_dir}/${stem}.log" ||
      fail 'pdfTeX-profile ifcsname rollback marker is absent'
    grep -q 'COUNTS=255,256,32767' "${run_dir}/${stem}.log" ||
      fail 'pdfTeX-profile extended-register boundary is absent'
    grep -q 'PENALTIES=20, 40,70, 80' "${run_dir}/${stem}.log" ||
      fail 'pdfTeX-profile penalty-array boundary is absent'
    grep -q 'SCANTOKENS-EOF' "${run_dir}/${stem}.log" ||
      fail 'pdfTeX-profile scantokens EOF boundary is absent'
  fi
}

run_etex_profile_recovery() {
  local executable="$1" profile="$2" status=0
  local run_dir="${out_dir}/etex-profile/${profile}-recovery"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$etex_profile_recovery_input" "${run_dir}/etex-profile-recovery.tex"
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C TZ=UTC \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/kpathsea" \
      "$executable" -ini -etex -interaction=batchmode etex-profile-recovery.tex \
      >terminal.txt 2>&1
  ) || status="$?"
  [[ "$status" -eq 1 ]] ||
    fail "$profile e-TeX recovery run exited with status $status instead of 1"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  local log="${run_dir}/etex-profile-recovery.log"
  grep -q 'Bad interaction mode (4)' "$log" &&
    grep -q 'INTERACTION-RECOVERED=0' "$log" ||
    fail "$profile interactionmode recovery evidence is absent"
  grep -q 'Extra \\middle' "$log" &&
    grep -q 'MIDDLE-MISSING-DELIMITER-RECOVERED' "$log" ||
    fail "$profile middle-delimiter recovery evidence is absent"
  grep -q '\\endL or \\endR problem (1 missing, 1 extra)' "$log" ||
    fail "$profile TeXXeT mismatch evidence is absent"
  grep -q 'SPLIT-FIRST=5.0pt' "$log" &&
    grep -q 'SPLIT-SECOND=0.0pt' "$log" ||
    fail "$profile destructive saved-discard splice evidence is absent"
  [[ -f "${run_dir}/etex-profile-recovery.dvi" &&
    -f "${run_dir}/etex-profile-recovery-effects.out" ]] ||
    fail "$profile e-TeX recovery artifacts are absent"
}

run_etex_profile_saved_hyph_codes() {
  local executable="$1" profile="$2" setup_status=0 status=0
  local run_dir="${out_dir}/etex-profile/${profile}-saved-hyph-codes"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$etex_profile_hyph_format_input" "${run_dir}/etex-profile-hyph-format.tex"
  cp "$etex_profile_hyph_input" "${run_dir}/etex-profile-hyph.tex"
  cp "$state_font_input" "${run_dir}/cmr10.tfm"
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C TZ=UTC \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/kpathsea" \
      "$executable" -ini -etex -interaction=batchmode etex-profile-hyph-format.tex \
      >format-terminal.txt 2>&1
  ) || setup_status="$?"
  [[ "$setup_status" -eq 0 ]] ||
    fail "$profile saved-hyphen-code format run exited with status $setup_status"
  if [[ -f "${run_dir}/pdftex14029-events.jsonl" ]]; then
    mv "${run_dir}/pdftex14029-events.jsonl" \
      "${run_dir}/format-events.jsonl"
  fi
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C TZ=UTC \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/kpathsea" \
      "$executable" -fmt=etex-profile-hyph-format -interaction=batchmode \
      etex-profile-hyph.tex >terminal.txt 2>&1
  ) || status="$?"
  [[ "$status" -eq 0 ]] ||
    fail "$profile saved-hyphen-code loaded run exited with status $status"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  grep -q 'UMBER-PDFTEX14029-SAVED-HYPH-CODES' \
    "${run_dir}/etex-profile-hyph.log" &&
    grep -q '@\\discretionary' "${run_dir}/etex-profile-hyph.log" &&
    grep -q 'SAVED-HYPH-CODES-PARAGRAPH-COMPLETE' \
      "${run_dir}/etex-profile-hyph.log" ||
    fail "$profile saved hyphen codes did not survive changed lccodes"
  [[ -f "${run_dir}/etex-profile-hyph.dvi" ]] ||
    fail "$profile saved-hyphen-code DVI is absent"
}

run_state() {
  local executable="$1" profile="$2"
  local run_dir="${out_dir}/state/${profile}" status=0
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$state_input" "${run_dir}/state.tex"
  cp "$state_font_input" "${run_dir}/cmr10.tfm"
  cp "$state_map_input" "${run_dir}/pdftex.map"
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C TZ=UTC \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/kpathsea" \
      "$executable" -ini -etex -interaction=batchmode state.tex \
      >terminal.txt 2>&1
  ) || status="$?"
  [[ "$status" -eq 0 ]] ||
    fail "$profile state run exited with status $status"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  [[ -f "${run_dir}/state.log" ]] ||
    fail "$profile state run did not write state.log"
  [[ -f "${run_dir}/state.pdf" ]] ||
    fail "$profile state run did not write state.pdf"
  [[ -f "${run_dir}/state-effects.out" ]] ||
    fail "$profile state run did not write state effects"
  grep -q 'UMBER-PDFTEX14029-COMMAND-STATE' "${run_dir}/state.log" ||
    fail "$profile state marker is absent"
  grep -q 'PDFTEX-TIMER-AVAILABLE' "${run_dir}/state.log" ||
    fail "$profile timer enquiry marker is absent"
  sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/state.log" \
    >"${run_dir}/ordinary.log"
  normalize_pdf "${run_dir}/state.pdf" "${run_dir}/normalized-pdf.txt"
}

record_linked_libraries() {
  local executable="$1"
  if command -v otool >/dev/null 2>&1; then
    otool -L "$executable" | sed -n '2,$p'
  elif command -v ldd >/dev/null 2>&1; then
    ldd "$executable"
  else
    printf 'unavailable\n'
  fi
}

write_build_record() {
  local record="${out_dir}/build-record.txt"
  local linker_path path tool tool_path profile executable mode output
  {
    printf 'identity pdftex14029-oracle-web2c-texlive-2026\n'
    printf 'engine pdfTeX\nengine-version 1.40.29\n'
    printf 'etex-version 2.6\ncharacter-profile eight-bit-exact\n'
    printf 'invocation-profile INITEX-with-etex-extensions\n'
    printf 'archive-url %s\narchive-sha512 %s\n' "$archive_url" "$archive_sha512"
    printf 'manifest-sha256 %s\n' "$(sha_digest 256 "$manifest")"
    printf 'source-date-epoch %s\n' "$source_date_epoch"
    printf 'configure ../src/configure --without-x --disable-shared --disable-all-pkgs --enable-pdftex --disable-synctex --disable-xetex --enable-missing -C CFLAGS=%q CXXFLAGS=%q\n' "$cflags" "$cxxflags"
    printf 'ordered-web-source-sha256 %s %s\n' \
      "${web_source#"${source_dir}/"}" "$(sha_digest 256 "$web_source")"
    for path in "${upstream_changes[@]}"; do
      printf 'ordered-change-sha256 %s %s\n' \
        "${path#"${source_dir}/"}" "$(sha_digest 256 "$path")"
    done
    printf 'instrumentation-change %s\n' \
      "${instrumentation_change#"${repo_root}/"}"
    printf 'instrumentation-change-sha256 %s\n' \
      "$(sha_digest 256 "$instrumentation_change")"
    printf 'extension-instrumentation-change %s\n' \
      "${extension_instrumentation_change#"${repo_root}/"}"
    printf 'extension-instrumentation-change-sha256 %s\n' \
      "$(sha_digest 256 "$extension_instrumentation_change")"
    printf 'state-instrumentation-change %s\n' \
      "${state_instrumentation_change#"${repo_root}/"}"
    printf 'state-instrumentation-change-sha256 %s\n' \
      "$(sha_digest 256 "$state_instrumentation_change")"
    printf 'extension-event-matrix-sha256 %s\n' \
      "$(sha_digest 256 "$extension_event_matrix")"
    printf 'etex-profile-input-sha256 %s\n' \
      "$(sha_digest 256 "$etex_profile_input")"
    printf 'etex-profile-compatibility-input-sha256 %s\n' \
      "$(sha_digest 256 "$etex_profile_compatibility_input")"
    printf 'etex-profile-recovery-input-sha256 %s\n' \
      "$(sha_digest 256 "$etex_profile_recovery_input")"
    printf 'etex-profile-hyph-format-input-sha256 %s\n' \
      "$(sha_digest 256 "$etex_profile_hyph_format_input")"
    printf 'etex-profile-hyph-input-sha256 %s\n' \
      "$(sha_digest 256 "$etex_profile_hyph_input")"
    printf 'etex-profile-boundary-matrix-sha256 %s\n' \
      "$(sha_digest 256 "$etex_profile_matrix")"
    printf 'state-event-matrix-sha256 %s\n' \
      "$(sha_digest 256 "$state_event_matrix")"
    printf 'extension-primitive-audit-sha256 %s\n' \
      "$(sha_digest 256 "$extension_primitive_audit")"
    printf 'tool-sha256 pdf-normalize %s\n' \
      "$(sha_digest 256 "$pdf_normalizer")"
    printf 'tool-sha256 tie %s\n' "$(sha_digest 256 "${web_build_dir}/tie")"
    printf 'tool-sha256 tangle %s\n' "$(sha_digest 256 "${web_build_dir}/tangle")"
    printf 'tool-sha256 tangle-pdftex14029 %s\n' \
      "$(sha_digest 256 "$wide_tangle")"
    printf 'generated-tangle-pdftex14029-source-sha256 %s\n' \
      "$(sha_digest 256 "${out_dir}/tangle-pdftex14029.c")"
    printf 'tool-sha256 web2c %s\n' \
      "$(sha_digest 256 "${web_build_dir}/web2c/web2c")"
    for tool in make cc c++; do
      tool_path="$(command -v "$tool")"
      printf 'host-tool-sha256 %s %s %s\n' \
        "$tool" "$tool_path" "$(sha_digest 256 "$tool_path")"
      printf 'host-tool-version %s %s\n' \
        "$tool" "$("$tool" --version 2>&1 | sed -n '1p')"
    done
    tool_path="$(command -v sh)"
    printf 'host-tool-sha256 sh %s %s\n' \
      "$tool_path" "$(sha_digest 256 "$tool_path")"
    linker_path="$(awk '$1 == "CXX" && $2 == "=" { print $3; exit }' \
      "${web_build_dir}/Makefile")"
    [[ -n "$linker_path" ]] || linker_path="$(command -v c++)"
    [[ "$linker_path" == /* ]] || linker_path="$(command -v "$linker_path")"
    printf 'host-tool-sha256 cxx-linker %s %s\n' \
      "$linker_path" "$(sha_digest 256 "$linker_path")"
    printf 'host-uname %s\n' "$(uname -a)"
    for path in \
      "${build_dir}/libs/libpng/libpng.a" \
      "${build_dir}/libs/zlib/libz.a" \
      "${build_dir}/libs/xpdf/libxpdf.a" \
      "${build_dir}/texk/kpathsea/.libs/libkpathsea.a"; do
      printf 'library-artifact-sha256 %s %s\n' \
        "${path#"${build_dir}/"}" "$(sha_digest 256 "$path")"
    done
    printf 'generated-clean-final-change-sha256 %s\n' \
      "$(sha_digest 256 "${out_dir}/clean-final.ch")"
    printf 'generated-instrumented-final-change-sha256 %s\n' \
      "$(sha_digest 256 "${out_dir}/instrumented-final.ch")"
    for profile in clean instrumented; do
      executable="$(profile_executable "$profile")"
      printf 'executable %s %s %s\n' "$profile" \
        "${executable#"${repo_root}/"}" "$(sha_digest 256 "$executable")"
      while IFS= read -r path; do
        printf 'linked-library %s %s\n' "$profile" "$path"
      done < <(record_linked_libraries "$executable")
      for mode in dvi pdf; do
        output="smoke-dvi.dvi"
        [[ "$mode" == dvi ]] || output="smoke-pdf.pdf"
        printf 'smoke-ordinary-log-sha256 %s %s %s\n' "$profile" "$mode" \
          "$(sha_digest 256 "${out_dir}/smoke/${profile}-${mode}/ordinary.log")"
        printf 'smoke-output-sha256 %s %s %s\n' "$profile" "$mode" \
          "$(sha_digest 256 "${out_dir}/smoke/${profile}-${mode}/${output}")"
        if [[ "$mode" == pdf ]]; then
          printf 'smoke-normalized-pdf-sha256 %s %s\n' "$profile" \
            "$(sha_digest 256 "${out_dir}/smoke/${profile}-${mode}/normalized-pdf.txt")"
        fi
      done
      printf 'transition-terminal-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/transitions/${profile}/terminal.txt")"
      printf 'transition-ordinary-log-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/transitions/${profile}/ordinary.log")"
      printf 'transition-dvi-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/transitions/${profile}/transitions.dvi")"
      printf 'transition-effect-output-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/transitions/${profile}/transitions-effects.out")"
      printf 'extension-terminal-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/extensions/${profile}/terminal.txt")"
      printf 'extension-ordinary-log-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/extensions/${profile}/ordinary.log")"
      printf 'extension-dvi-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/extensions/${profile}/extensions.dvi")"
      printf 'extension-effect-output-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/extensions/${profile}/extensions-effects.out")"
      printf 'state-terminal-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/state/${profile}/terminal.txt")"
      printf 'state-ordinary-log-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/state/${profile}/ordinary.log")"
      printf 'state-pdf-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/state/${profile}/state.pdf")"
      printf 'state-normalized-pdf-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/state/${profile}/normalized-pdf.txt")"
      printf 'state-effect-output-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/state/${profile}/state-effects.out")"
      if [[ "$profile" == instrumented ]]; then
        printf 'transition-trace-sha256 %s %s\n' "$profile" \
          "$(sha_digest 256 "${out_dir}/transitions/${profile}/pdftex14029-events.jsonl")"
        printf 'extension-trace-sha256 %s %s\n' "$profile" \
          "$(sha_digest 256 "${out_dir}/extensions/${profile}/pdftex14029-events.jsonl")"
        printf 'state-trace-sha256 %s %s\n' "$profile" \
          "$(sha_digest 256 "${out_dir}/state/${profile}/pdftex14029-events.jsonl")"
      fi
    done
  } > "$record"
}

mkdir -p "$bin_dir"
fetch_source
extract_source
configure_tools
build_wide_tangle
build_variant "$(profile_executable clean)"
cp "${web_build_dir}/pdftex-final.ch" "${out_dir}/clean-final.ch"
build_variant "$(profile_executable instrumented)" \
  "$instrumentation_change" "$extension_instrumentation_change" \
  "$state_instrumentation_change"
cp "${web_build_dir}/pdftex-final.ch" "${out_dir}/instrumented-final.ch"

audit_extension_primitives() {
  local pdf_inventory shared_inventory extension_inventory audit_inventory
  local primitive owner phase gate seam extra pattern
  pdf_inventory="$(mktemp)"
  shared_inventory="$(mktemp)"
  extension_inventory="$(mktemp)"
  audit_inventory="$(mktemp)"
  awk 'match($0,/primitive\("[^"]+"/) {
    print substr($0,RSTART+11,RLENGTH-12)
  }' "$web_source" | LC_ALL=C sort -u >"$pdf_inventory"
  {
    awk 'match($0,/primitive\("[^"]+"/) {
      print substr($0,RSTART+11,RLENGTH-12)
    }' "$tex_web_source"
    awk 'match($0,/primitive\("[^"]+"/) {
      print substr($0,RSTART+11,RLENGTH-12)
    }' "$etex_change_source"
  } | LC_ALL=C sort -u >"$shared_inventory"
  # Both inventories are `LC_ALL=C sort -u`ed above, so the comparison has to
  # use the same collation. Without this the script aborts under any ordinary
  # UTF-8 locale with "comm: input is not in sorted order", which is why it
  # only ever ran clean in a C environment.
  LC_ALL=C comm -23 "$pdf_inventory" "$shared_inventory" >"$extension_inventory"
  while IFS='|' read -r primitive owner phase gate seam extra; do
    [[ -z "$primitive" || "$primitive" == \#* ]] && continue
    [[ -n "$owner" && -n "$phase" && -n "$gate" && -n "$seam" &&
      -z "${extra:-}" ]] ||
      fail "malformed extension primitive audit row for ${primitive:-unknown}"
    [[ "$owner" == command-core || "$owner" == executor-backend ]] ||
      fail "unknown extension primitive owner for $primitive: $owner"
    if [[ "$owner" == command-core && "$phase" == expansion ]]; then
      awk -F'|' -v primitive="$primitive" -v gate="$gate" \
        '$2 == primitive && $3 == gate { found=1 } END { exit !found }' \
        "$extension_event_matrix" ||
        fail "command-core expansion primitive $primitive has no matrix boundary: $gate"
    elif [[ "$owner" == command-core && "$phase" == state ]]; then
      grep -Fq "\\${primitive}" "$state_input" ||
        fail "command-core state primitive $primitive is absent from state.tex"
      if [[ "$gate" == "phase-2 enquiry matrix" ]]; then
        awk -F'|' -v primitive="$primitive" \
          '$2 == primitive { found=1 } END { exit !found }' \
          "$state_event_matrix" ||
          fail "command-core enquiry primitive $primitive has no state matrix row"
      fi
    fi
    printf '%s\n' "$primitive"
  done <"$extension_primitive_audit" | LC_ALL=C sort -u >"$audit_inventory"
  cmp "$extension_inventory" "$audit_inventory" >/dev/null ||
    fail "extension primitive audit does not exactly cover canonical pdfTeX additions"
  while IFS='|' read -r family primitive boundary fixture seam pattern extra; do
    [[ -z "$family" || "$family" == \#* ]] && continue
    [[ -n "$primitive" && -n "$boundary" && -n "$fixture" && -n "$seam" &&
      -n "$pattern" && -z "${extra:-}" ]] ||
      fail "malformed extension event matrix row for ${family:-unknown}"
    awk -F'|' -v primitive="$primitive" \
      '$1 == primitive && $2 == "command-core" && $3 == "expansion" {
       found=1 } END { exit !found }' \
      "$extension_primitive_audit" ||
      fail "extension matrix boundary is not owned by command-core expansion primitive $primitive: $boundary"
  done <"$extension_event_matrix"
  [[ "$(wc -l <"$pdf_inventory" | tr -d ' ')" -eq 549 ]] ||
    fail "canonical pdfTeX primitive inventory no longer contains 549 entries"
  [[ "$(wc -l <"$shared_inventory" | tr -d ' ')" -eq 391 ]] ||
    fail "canonical shared TeX/e-TeX primitive inventory no longer contains 391 entries"
  rm -f "$pdf_inventory" "$shared_inventory" "$extension_inventory" \
    "$audit_inventory"
}

audit_extension_primitives
for profile in clean instrumented; do
  for mode in dvi pdf; do
    run_smoke "$(profile_executable "$profile")" "$profile" "$mode"
  done
  run_transitions "$(profile_executable "$profile")" "$profile"
  run_extensions "$(profile_executable "$profile")" "$profile"
  run_state "$(profile_executable "$profile")" "$profile"
  run_etex_profile_boundaries "$(profile_executable "$profile")" "${profile}-extended"
  run_etex_profile_boundaries "$(profile_executable "$profile")" "${profile}-compatibility"
  run_etex_profile_recovery "$(profile_executable "$profile")" "$profile"
  run_etex_profile_saved_hyph_codes "$(profile_executable "$profile")" "$profile"
done
compare_smoke_channels dvi
compare_smoke_channels pdf
compare_channels "independently normalized PDF smoke oracle" \
  "${out_dir}/smoke/clean-pdf" "${out_dir}/smoke/instrumented-pdf" \
  normalized-pdf.txt
compare_channels "transition oracle" \
  "${out_dir}/transitions/clean" "${out_dir}/transitions/instrumented" \
  terminal.txt ordinary.log status.txt transitions.dvi transitions-effects.out
compare_channels "extension oracle" \
  "${out_dir}/extensions/clean" "${out_dir}/extensions/instrumented" \
  terminal.txt ordinary.log status.txt extensions.dvi extensions-effects.out
compare_channels "state oracle" \
  "${out_dir}/state/clean" "${out_dir}/state/instrumented" \
  terminal.txt ordinary.log status.txt state.pdf state-effects.out
compare_channels "extended pdfTeX-profile e-TeX boundary oracle" \
  "${out_dir}/etex-profile/clean-extended" \
  "${out_dir}/etex-profile/instrumented-extended" \
  terminal.txt status.txt etex-profile-boundaries.log \
  etex-profile-boundaries.dvi etex-profile-boundaries-effects.out
compare_channels "compatibility pdfTeX-profile boundary oracle" \
  "${out_dir}/etex-profile/clean-compatibility" \
  "${out_dir}/etex-profile/instrumented-compatibility" \
  terminal.txt status.txt etex-profile-compatibility.log \
  etex-profile-compatibility.dvi
compare_channels "pdfTeX-profile e-TeX recovery oracle" \
  "${out_dir}/etex-profile/clean-recovery" \
  "${out_dir}/etex-profile/instrumented-recovery" \
  terminal.txt status.txt etex-profile-recovery.log \
  etex-profile-recovery.dvi etex-profile-recovery-effects.out
compare_channels "pdfTeX-profile saved-hyphen-code oracle" \
  "${out_dir}/etex-profile/clean-saved-hyph-codes" \
  "${out_dir}/etex-profile/instrumented-saved-hyph-codes" \
  terminal.txt status.txt etex-profile-hyph.log etex-profile-hyph.dvi
compare_channels "independently normalized PDF state oracle" \
  "${out_dir}/state/clean" "${out_dir}/state/instrumented" \
  normalized-pdf.txt
trace="${out_dir}/transitions/instrumented/pdftex14029-events.jsonl"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$trace"
while IFS='|' read -r family boundary fixture seam pattern extra; do
  [[ -z "$family" || "$family" == \#* ]] && continue
  [[ -n "$boundary" && -n "$fixture" && -n "$seam" && -n "$pattern" &&
    -z "${extra:-}" ]] ||
    fail "malformed semantic event matrix row for ${family:-unknown}"
  grep -Fq "$pattern" "$trace" ||
    fail "trace is missing $family/$boundary from $fixture at $seam"
done <"$semantic_event_matrix"
extension_trace="${out_dir}/extensions/instrumented/pdftex14029-events.jsonl"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$extension_trace"
while IFS='|' read -r family primitive boundary fixture seam pattern extra; do
  [[ -z "$family" || "$family" == \#* ]] && continue
  [[ -n "$primitive" && -n "$boundary" && -n "$fixture" && -n "$seam" &&
    -n "$pattern" && -z "${extra:-}" ]] ||
    fail "malformed extension event matrix row for ${family:-unknown}"
  grep -Fq "$pattern" "$extension_trace" ||
    fail "extension trace is missing $family/$primitive/$boundary from $fixture at $seam"
done <"$extension_event_matrix"
etex_profile_trace="${out_dir}/etex-profile/instrumented-extended/pdftex14029-events.jsonl"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$etex_profile_trace"
etex_profile_recovery_trace="${out_dir}/etex-profile/instrumented-recovery/pdftex14029-events.jsonl"
etex_profile_hyph_format_trace="${out_dir}/etex-profile/instrumented-saved-hyph-codes/format-events.jsonl"
etex_profile_hyph_trace="${out_dir}/etex-profile/instrumented-saved-hyph-codes/pdftex14029-events.jsonl"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$etex_profile_recovery_trace"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$etex_profile_hyph_format_trace"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$etex_profile_hyph_trace"
while IFS='|' read -r family boundary pattern compatibility extra; do
  [[ -z "$family" || "$family" == \#* ]] && continue
  [[ -n "$boundary" && -n "$pattern" && "$compatibility" == absent &&
    -z "${extra:-}" ]] ||
    fail "malformed pdfTeX-profile boundary matrix row for ${family:-unknown}"
  grep -Fq "$pattern" "$etex_profile_trace" \
    "$etex_profile_recovery_trace" "$etex_profile_hyph_format_trace" \
    "$etex_profile_hyph_trace" ||
    fail "pdfTeX extended profile is missing $family/$boundary"
  if [[ -f "${out_dir}/etex-profile/instrumented-compatibility/pdftex14029-events.jsonl" ]]; then
    ! grep -Fq "$pattern" \
      "${out_dir}/etex-profile/instrumented-compatibility/pdftex14029-events.jsonl" ||
      fail "pdfTeX compatibility profile unexpectedly emitted $family/$boundary"
  fi
done <"$etex_profile_matrix"
state_trace="${out_dir}/state/instrumented/pdftex14029-events.jsonl"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$state_trace"
while IFS='|' read -r family primitive boundary fixture seam pattern extra; do
  [[ -z "$family" || "$family" == \#* ]] && continue
  [[ -n "$primitive" && -n "$boundary" && -n "$fixture" && -n "$seam" &&
    -n "$pattern" && -z "${extra:-}" ]] ||
    fail "malformed state event matrix row for ${family:-unknown}"
  grep -Fq "$pattern" "$state_trace" ||
    fail "state trace is missing $family/$primitive/$boundary from $fixture at $seam"
done <"$state_event_matrix"
while IFS='|' read -r primitive owner phase gate seam extra; do
  [[ -z "$primitive" || "$primitive" == \#* ]] && continue
  [[ "$owner" == command-core && "$phase" == state ]] || continue
  grep -Fq "\"control_sequence\":\"$primitive\"" "$state_trace" ||
    fail "state trace did not deliver command-core primitive $primitive"
  if [[ "$gate" == "phase-2 command-state matrix" &&
    "$primitive" != pdfoptionpdfminorversion ]]; then
    pattern="\"value\":\"$primitive"
    grep -Fq "$pattern" "$state_trace" ||
      fail "state trace did not commit semantic state for $primitive"
  fi
done <"$extension_primitive_audit"
run_transitions "$(profile_executable instrumented)" instrumented-repeat
run_extensions "$(profile_executable instrumented)" instrumented-repeat
run_state "$(profile_executable instrumented)" instrumented-repeat
run_etex_profile_boundaries "$(profile_executable instrumented)" instrumented-repeat-extended
run_etex_profile_boundaries "$(profile_executable instrumented)" instrumented-repeat-compatibility
run_etex_profile_recovery "$(profile_executable instrumented)" instrumented-repeat
run_etex_profile_saved_hyph_codes "$(profile_executable instrumented)" instrumented-repeat
compare_channels "repeated instrumented transition oracle" \
  "${out_dir}/transitions/instrumented" \
  "${out_dir}/transitions/instrumented-repeat" \
  terminal.txt ordinary.log status.txt transitions.dvi transitions-effects.out \
  pdftex14029-events.jsonl
compare_channels "repeated instrumented extension oracle" \
  "${out_dir}/extensions/instrumented" \
  "${out_dir}/extensions/instrumented-repeat" \
  terminal.txt ordinary.log status.txt extensions.dvi extensions-effects.out \
  pdftex14029-events.jsonl
compare_channels "repeated instrumented state oracle" \
  "${out_dir}/state/instrumented" \
  "${out_dir}/state/instrumented-repeat" \
  terminal.txt ordinary.log status.txt state.pdf normalized-pdf.txt state-effects.out \
  pdftex14029-events.jsonl
compare_channels "repeated extended pdfTeX-profile e-TeX boundary oracle" \
  "${out_dir}/etex-profile/instrumented-extended" \
  "${out_dir}/etex-profile/instrumented-repeat-extended" \
  terminal.txt status.txt etex-profile-boundaries.log \
  etex-profile-boundaries.dvi etex-profile-boundaries-effects.out \
  pdftex14029-events.jsonl
compare_channels "repeated compatibility pdfTeX-profile boundary oracle" \
  "${out_dir}/etex-profile/instrumented-compatibility" \
  "${out_dir}/etex-profile/instrumented-repeat-compatibility" \
  terminal.txt status.txt etex-profile-compatibility.log \
  etex-profile-compatibility.dvi pdftex14029-events.jsonl
compare_channels "repeated pdfTeX-profile e-TeX recovery oracle" \
  "${out_dir}/etex-profile/instrumented-recovery" \
  "${out_dir}/etex-profile/instrumented-repeat-recovery" \
  terminal.txt status.txt etex-profile-recovery.log \
  etex-profile-recovery.dvi etex-profile-recovery-effects.out \
  pdftex14029-events.jsonl
compare_channels "repeated pdfTeX-profile saved-hyphen-code oracle" \
  "${out_dir}/etex-profile/instrumented-saved-hyph-codes" \
  "${out_dir}/etex-profile/instrumented-repeat-saved-hyph-codes" \
  terminal.txt status.txt etex-profile-hyph.log etex-profile-hyph.dvi \
  format-events.jsonl pdftex14029-events.jsonl
write_build_record
printf '%s\n' "$bin_dir"
