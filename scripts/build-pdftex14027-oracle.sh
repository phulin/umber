#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

manifest="tests/pdftex14027-oracle-manifest.txt"
source_name="texlive-20250308-source.tar.xz"
cache_root="${UMBER_REF_TEXLIVE_SOURCE:-third_party/texlive-source}"
[[ "$cache_root" == /* ]] || cache_root="${repo_root}/${cache_root}"
source_tar="${cache_root}/${source_name}"
source_dir="${cache_root}/src"
build_dir="${cache_root}/build-pdftex14027"
web_source_dir="${source_dir}/texk/web2c"
web_build_dir="${build_dir}/texk/web2c"
target_dir="${CARGO_TARGET_DIR:-target}"
[[ "$target_dir" == /* ]] || target_dir="${repo_root}/${target_dir}"
out_dir="${target_dir}/pdftex14027-oracle"
bin_dir="${out_dir}/bin"
instrumentation_change="${UMBER_PDFTEX14027_INSTRUMENTATION_CHANGE:-${repo_root}/tests/pdftex14027-oracle/instrumentation.ch}"
dvi_input="${repo_root}/tests/pdftex14027-oracle/smoke-dvi.tex"
pdf_input="${repo_root}/tests/pdftex14027-oracle/smoke-pdf.tex"
transition_input="${repo_root}/tests/pdftex14027-oracle/transitions.tex"
transition_child="${repo_root}/tests/pdftex14027-oracle/transitions-child.tex"
semantic_event_matrix="${repo_root}/tests/pdftex14027-oracle/semantic-event-matrix.txt"
wide_tangle="${out_dir}/tangle-pdftex14027"
cflags="-O2"
cxxflags="-O2"
source_date_epoch="${SOURCE_DATE_EPOCH:-1783604160}"
offline=0

usage() {
  cat <<'EOF'
usage: scripts/build-pdftex14027-oracle.sh [--offline]

Acquire and verify the pinned TeX Live 2025 source snapshot, then build
canonical pdfTeX 1.40.27 clean and instrumented eight-bit Web2C executables.
The repository-owned final change is applied only to the latter and emits the
shared schema-v1 command trace.

Outputs and a complete identity record are written under
target/pdftex14027-oracle. After the first acquisition, --offline performs no
network I/O.
EOF
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --offline) offline=1 ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'build-pdftex14027-oracle: unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

fail() {
  printf 'build-pdftex14027-oracle: %s\n' "$*" >&2
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

archive_url="$(awk '$1 == "archive" { print $2 }' "$manifest")"
archive_sha512="$(awk '$1 == "archive" { print $3 }' "$manifest")"
[[ -n "$archive_url" && -n "$archive_sha512" ]] ||
  fail "missing archive pin in $manifest"

verify_archive() {
  local actual
  actual="$(sha_digest 512 "$source_tar")"
  [[ "$actual" == "$archive_sha512" ]] ||
    fail "sha512 mismatch for $source_tar: expected $archive_sha512, got $actual"
}

fetch_source() {
  mkdir -p "$cache_root"
  if [[ -f "$source_tar" ]]; then
    verify_archive
    printf 'verified %s\n' "$source_tar" >&2
    return
  fi
  [[ "$offline" -eq 0 ]] || fail "missing $source_tar while running --offline"
  [[ "$cache_root" == "${repo_root}/third_party/texlive-source" ]] ||
    fail "selected TeX Live cache is incomplete: $cache_root"
  local tmp="${source_tar}.tmp"
  printf 'fetching %s\n' "$archive_url" >&2
  curl -fL "$archive_url" -o "$tmp"
  mv "$tmp" "$source_tar"
  verify_archive
}

verify_inputs() {
  local kind path expected extra actual root
  while read -r kind path expected extra; do
    [[ -z "${kind:-}" || "$kind" == \#* || "$kind" == archive ]] && continue
    [[ -z "${extra:-}" ]] ||
      fail "malformed input pin: $kind $path $expected $extra"
    case "$kind" in
      source-sha256) root="$source_dir" ;;
      repository-sha256) root="$repo_root" ;;
      *) fail "unknown manifest record kind: $kind" ;;
    esac
    [[ -f "${root}/${path}" ]] ||
      fail "missing pinned input ${root}/${path}"
    actual="$(sha_digest 256 "${root}/${path}")"
    [[ "$actual" == "$expected" ]] ||
      fail "sha256 mismatch for $path: expected $expected, got $actual"
  done < "$manifest"
  [[ -f "$instrumentation_change" ]] ||
    fail "missing instrumentation change file: $instrumentation_change"
}

extract_source() {
  if [[ ! -f "${source_dir}/configure" ]]; then
    rm -rf "$source_dir"
    mkdir -p "$source_dir"
    tar -xJf "$source_tar" -C "$source_dir" --strip-components=1
  fi
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
  local generated="${out_dir}/tangle-pdftex14027.c"
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
  printf '%s/umber-pdftex14027-oracle-%s' "$bin_dir" "$profile"
}

run_smoke() {
  local executable="$1" profile="$2" mode="$3" input output marker status=0
  local run_dir="${out_dir}/smoke/${profile}-${mode}"
  input="$dvi_input"
  output="smoke-dvi.dvi"
  marker="UMBER-PDFTEX14027-ORACLE-DVI-SMOKE"
  [[ "$mode" == dvi ]] || {
    input="$pdf_input"
    output="smoke-pdf.pdf"
    marker="UMBER-PDFTEX14027-ORACLE-PDF-SMOKE"
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
  grep -q 'pdfTeX, Version 3.141592653-2.6-1.40.27' "${run_dir}/smoke-${mode}.log" ||
    fail "$profile/$mode did not identify canonical pdfTeX 1.40.27"
  grep -q "$marker" "${run_dir}/smoke-${mode}.log" ||
    fail "$profile/$mode smoke marker is absent"
  grep -q 'PDFTEX=14027' "${run_dir}/smoke-${mode}.log" ||
    fail "$profile/$mode pdfTeX version primitive marker is absent"
  grep -q 'ETEX=2.6' "${run_dir}/smoke-${mode}.log" ||
    fail "$profile/$mode e-TeX version primitive marker is absent"
  grep -q '=42' "${run_dir}/smoke-${mode}.log" ||
    fail "$profile/$mode arithmetic marker is absent"
  sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/smoke-${mode}.log" \
    >"${run_dir}/ordinary.log"
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
    printf 'identity pdftex14027-oracle-web2c-texlive-2025\n'
    printf 'engine pdfTeX\nengine-version 1.40.27\n'
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
    printf 'tool-sha256 tie %s\n' "$(sha_digest 256 "${web_build_dir}/tie")"
    printf 'tool-sha256 tangle %s\n' "$(sha_digest 256 "${web_build_dir}/tangle")"
    printf 'tool-sha256 tangle-pdftex14027 %s\n' \
      "$(sha_digest 256 "$wide_tangle")"
    printf 'generated-tangle-pdftex14027-source-sha256 %s\n' \
      "$(sha_digest 256 "${out_dir}/tangle-pdftex14027.c")"
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
      done
      printf 'transition-terminal-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/transitions/${profile}/terminal.txt")"
      printf 'transition-ordinary-log-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/transitions/${profile}/ordinary.log")"
      printf 'transition-dvi-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/transitions/${profile}/transitions.dvi")"
      printf 'transition-effect-output-sha256 %s %s\n' "$profile" \
        "$(sha_digest 256 "${out_dir}/transitions/${profile}/transitions-effects.out")"
      if [[ "$profile" == instrumented ]]; then
        printf 'transition-trace-sha256 %s %s\n' "$profile" \
          "$(sha_digest 256 "${out_dir}/transitions/${profile}/pdftex14027-events.jsonl")"
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
build_variant "$(profile_executable instrumented)" "$instrumentation_change"
cp "${web_build_dir}/pdftex-final.ch" "${out_dir}/instrumented-final.ch"

for profile in clean instrumented; do
  for mode in dvi pdf; do
    run_smoke "$(profile_executable "$profile")" "$profile" "$mode"
  done
  run_transitions "$(profile_executable "$profile")" "$profile"
done
compare_smoke_channels dvi
compare_smoke_channels pdf
compare_channels "transition oracle" \
  "${out_dir}/transitions/clean" "${out_dir}/transitions/instrumented" \
  terminal.txt ordinary.log status.txt transitions.dvi transitions-effects.out
trace="${out_dir}/transitions/instrumented/pdftex14027-events.jsonl"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$trace"
while IFS='|' read -r family boundary fixture seam pattern extra; do
  [[ -z "$family" || "$family" == \#* ]] && continue
  [[ -n "$boundary" && -n "$fixture" && -n "$seam" && -n "$pattern" &&
    -z "${extra:-}" ]] ||
    fail "malformed semantic event matrix row for ${family:-unknown}"
  grep -Fq "$pattern" "$trace" ||
    fail "trace is missing $family/$boundary from $fixture at $seam"
done <"$semantic_event_matrix"
run_transitions "$(profile_executable instrumented)" instrumented-repeat
compare_channels "repeated instrumented transition oracle" \
  "${out_dir}/transitions/instrumented" \
  "${out_dir}/transitions/instrumented-repeat" \
  terminal.txt ordinary.log status.txt transitions.dvi transitions-effects.out \
  pdftex14027-events.jsonl
write_build_record
printf '%s\n' "$bin_dir"
