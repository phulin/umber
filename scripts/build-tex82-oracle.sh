#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

manifest="tests/tex82-oracle-manifest.txt"
source_name="texlive-20250308-source.tar.xz"
cache_root="${UMBER_REF_TEXLIVE_SOURCE:-third_party/texlive-source}"
[[ "$cache_root" == /* ]] || cache_root="${repo_root}/${cache_root}"
source_tar="${cache_root}/${source_name}"
source_dir="${cache_root}/src"
build_dir="${cache_root}/build"
web_source_dir="${source_dir}/texk/web2c"
web_build_dir="${build_dir}/texk/web2c"
target_dir="${CARGO_TARGET_DIR:-target}"
[[ "$target_dir" == /* ]] || target_dir="${repo_root}/${target_dir}"
out_dir="${target_dir}/tex82-oracle"
bin_dir="${out_dir}/bin"
clean_executable="${bin_dir}/umber-tex82-oracle"
instrumentable_executable="${bin_dir}/umber-tex82-oracle-instrumentable"
instrumentation_change="${UMBER_TEX82_INSTRUMENTATION_CHANGE:-${repo_root}/tests/tex82-oracle/instrumentation.ch}"
smoke_input="${repo_root}/tests/tex82-oracle/smoke.tex"
transition_input="${repo_root}/tests/tex82-oracle/transitions.tex"
transition_child="${repo_root}/tests/tex82-oracle/transitions-child.tex"
transition_support=(
  "${repo_root}/tests/tex82-oracle/expansion-macros.tex"
  "${repo_root}/tests/tex82-oracle/alignment-delivery.tex"
  "${repo_root}/tests/tex82-oracle/scanner-conditionals.tex"
  "${repo_root}/tests/tex82-oracle/scanner-conditionals-eof.tex"
  "${repo_root}/tests/tex82-oracle/input-recovery.tex"
  "${repo_root}/tests/tex82-oracle/input-eof-normal.tex"
  "${repo_root}/tests/tex82-oracle/input-eof-defining.tex"
  "${repo_root}/tests/tex82-oracle/input-eof-matching.tex"
  "${repo_root}/tests/tex82-oracle/input-eof-absorbing.tex"
  "${repo_root}/tests/tex82-oracle/input-eof-aligning.tex"
)
semantic_event_matrix="${repo_root}/tests/tex82-oracle/semantic-event-matrix.txt"
cflags="-O2"
cxxflags="-O2"
source_date_epoch="${SOURCE_DATE_EPOCH:-1783604160}"
offline=0

usage() {
  cat <<'EOF'
usage: scripts/build-tex82-oracle.sh [--offline]

Acquire and verify the pinned TeX Live 2025 source snapshot, then build the
canonical TeX82 Web2C oracle twice: once from the ordered upstream change
stack and once with a final repository-owned instrumentation change file.
The default final change emits schema-v1 command-core transitions. Set
UMBER_TEX82_INSTRUMENTATION_CHANGE to select another final change file.

Outputs and a complete identity record are written under target/tex82-oracle.
After the first acquisition, --offline performs no network I/O.
EOF
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --offline) offline=1 ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'build-tex82-oracle: unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

fail() {
  printf 'build-tex82-oracle: %s\n' "$*" >&2
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
[[ -n "$archive_url" && -n "$archive_sha512" ]] || fail "missing archive pin in $manifest"

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
    [[ -z "${extra:-}" ]] || fail "malformed input pin: $kind $path $expected $extra"
    case "$kind" in
      source-sha256) root="$source_dir" ;;
      repository-sha256) root="$repo_root" ;;
      *) fail "unknown manifest record kind: $kind" ;;
    esac
    [[ -f "${root}/${path}" ]] || fail "missing pinned input ${root}/${path}"
    actual="$(sha_digest 256 "${root}/${path}")"
    [[ "$actual" == "$expected" ]] ||
      fail "sha256 mismatch for $path: expected $expected, got $actual"
  done < "$manifest"
  [[ -f "$instrumentation_change" ]] ||
    fail "missing instrumentation change file: $instrumentation_change"
  [[ -f "$semantic_event_matrix" ]] ||
    fail "missing semantic event matrix: $semantic_event_matrix"
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
        --enable-tex --disable-synctex --disable-xetex --enable-missing -C \
        CFLAGS="$cflags" CXXFLAGS="$cxxflags"
    )
  fi
  if [[ ! -f "${web_build_dir}/Makefile" ]]; then
    make -C "$build_dir"
  fi
  make -C "$web_build_dir" tie tangle web2c/web2c
}

upstream_changes=(
  "${web_source_dir}/tex.web"
  "${web_source_dir}/tex.ch"
  "${web_source_dir}/enctexdir/enctex1.ch"
  "${web_source_dir}/enctexdir/enctex-tex.ch"
  "${web_source_dir}/enctexdir/enctex2.ch"
  "${web_source_dir}/tex-binpool.ch"
)

reset_tex_products() {
  rm -f \
    "${web_build_dir}/tex-final.ch" \
    "${web_build_dir}/tex.p" \
    "${web_build_dir}/tex.pool" \
    "${web_build_dir}/tex-web2c" \
    "${web_build_dir}/texini.c" \
    "${web_build_dir}/tex0.c" \
    "${web_build_dir}/texcoerce.h" \
    "${web_build_dir}/texd.h" \
    "${web_build_dir}/tex-pool.c" \
    "${web_build_dir}/tex-texini.o" \
    "${web_build_dir}/tex-tex0.o" \
    "${web_build_dir}/tex-texextra.o" \
    "${web_build_dir}/tex-tex-pool.o" \
    "${web_build_dir}/tex"
}

build_variant() {
  local destination="$1"
  shift
  local change_stack=("${upstream_changes[@]}" "$@")
  reset_tex_products
  (
    cd "$web_build_dir"
    TEXMFCNF="${source_dir}/texk/kpathsea" WEBINPUTS=".:${web_source_dir}" \
      ./tie -c tex-final.ch "${change_stack[@]}"
    TEXMFCNF="${source_dir}/texk/kpathsea" WEBINPUTS=".:${web_source_dir}" \
      ./tangle tex tex-final
    AM_V_P=false ./web2c-sh tex-web2c tex
  )
  make -C "$web_build_dir" tex
  [[ -x "${web_build_dir}/tex" ]] || fail "TeX executable was not built"
  cp "${web_build_dir}/tex" "$destination"
  chmod +x "$destination"
}

run_smoke() {
  local executable="$1" variant="$2" run_dir
  run_dir="${out_dir}/smoke/${variant}"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$smoke_input" "${run_dir}/smoke.tex"
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/web2c/triptrap" \
      "$executable" -ini -interaction=batchmode smoke.tex >terminal.txt 2>&1
  )
  [[ -f "${run_dir}/smoke.log" ]] || fail "$variant smoke run did not write smoke.log"
  grep -q 'UMBER-TEX82-ORACLE-SMOKE' "${run_dir}/smoke.log" ||
    fail "$variant smoke marker is absent"
  grep -q 'COUNT=42' "${run_dir}/smoke.log" ||
    fail "$variant arithmetic smoke marker is absent"
  sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/smoke.log" \
    > "${run_dir}/ordinary.log"
}

run_transitions() {
  local executable="$1" variant="$2" run_dir status=0 support
  run_dir="${out_dir}/transitions/${variant}"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$transition_input" "${run_dir}/transitions.tex"
  cp "$transition_child" "${run_dir}/transitions-child.tex"
  for support in "${transition_support[@]}"; do
    cp "$support" "$run_dir/"
  done
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/web2c/triptrap" \
      "$executable" -ini -interaction=nonstopmode transitions.tex \
      >terminal.txt 2>&1
  ) || status="$?"
  [[ "$status" -le 1 ]] ||
    fail "$variant transition run exited with unexpected status $status"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  [[ -f "${run_dir}/transitions.log" ]] ||
    fail "$variant transition run did not write transitions.log"
  [[ -f "${run_dir}/transitions.dvi" ]] ||
    fail "$variant transition run did not write transitions.dvi"
  [[ -f "${run_dir}/transitions-effects.out" ]] ||
    fail "$variant transition run did not write transitions-effects.out"
  grep -q 'UMBER-TEX82-TRANSITIONS' "${run_dir}/transitions.log" ||
    fail "$variant transition marker is absent"
  for marker in \
    'UMBER-TEX82-ALIGNMENT-BEGIN' \
    'UMBER-TEX82-U-TEMPLATE' \
    'UMBER-TEX82-V-TEMPLATE' \
    'UMBER-TEX82-NOALIGN' \
    'UMBER-TEX82-ALIGNMENT-END' \
    '! Missing # inserted in alignment preamble.' \
    'You have given more \span or & marks than there were' \
    '! Missing { inserted.' \
    '! Missing } inserted.' \
    'UMBER-TEX82-SCANNERS: I=-83' \
    'UMBER-TEX82-IF=TRUE' \
    'UMBER-TEX82-IFCAT=TRUE' \
    'UMBER-TEX82-IFX=TRUE' \
    'UMBER-TEX82-SKIP=KEPT' \
    'UMBER-TEX82-CASE=TWO' \
    'UMBER-TEX82-NESTED-EVALUATION=TRUE' \
    'UMBER-TEX82-LIMIT-RECOVERY' \
    'UMBER-TEX82-SKIP-RECOVERY' \
    '! Extra \or.' \
    '! Incomplete \iffalse; all text was ignored after line 1.'; do
    grep -Fq "$marker" "${run_dir}/transitions.log" ||
      fail "$variant scanner/conditional observation is absent: $marker"
  done
  sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/transitions.log" \
    >"${run_dir}/ordinary.log"
}

write_build_record() {
  local record="${out_dir}/build-record.txt" linker_path path tool tool_path
  {
    printf 'identity tex82-oracle-web2c-texlive-2025\n'
    printf 'engine TeX\nengine-version 3.141592653\n'
    printf 'character-profile eight-bit-exact\ninvocation-profile INITEX\n'
    printf 'archive-url %s\narchive-sha512 %s\n' "$archive_url" "$archive_sha512"
    printf 'manifest-sha256 %s\n' "$(sha_digest 256 "$manifest")"
    printf 'source-date-epoch %s\n' "$source_date_epoch"
    printf 'configure ../src/configure --without-x --disable-shared --disable-all-pkgs --enable-tex --disable-synctex --disable-xetex --enable-missing -C CFLAGS=%q CXXFLAGS=%q\n' "$cflags" "$cxxflags"
    for path in "${upstream_changes[@]}"; do
      printf 'ordered-change-sha256 %s %s\n' \
        "${path#"${source_dir}/"}" "$(sha_digest 256 "$path")"
    done
    printf 'instrumentation-change %s\n' "${instrumentation_change#"${repo_root}/"}"
    printf 'instrumentation-change-sha256 %s\n' "$(sha_digest 256 "$instrumentation_change")"
    printf 'tool-sha256 tie %s\n' "$(sha_digest 256 "${web_build_dir}/tie")"
    printf 'tool-sha256 tangle %s\n' "$(sha_digest 256 "${web_build_dir}/tangle")"
    printf 'tool-sha256 web2c %s\n' \
      "$(sha_digest 256 "${web_build_dir}/web2c/web2c")"
    for tool in make gcc g++; do
      tool_path="$(command -v "$tool")"
      printf 'host-tool-sha256 %s %s %s\n' \
        "$tool" "$tool_path" "$(sha_digest 256 "$tool_path")"
      printf 'host-tool-version %s %s\n' \
        "$tool" "$("$tool" --version 2>&1 | sed -n '1p')"
    done
    tool_path="$(command -v sh)"
    printf 'host-tool-sha256 sh %s %s\n' \
      "$tool_path" "$(sha_digest 256 "$tool_path")"
    linker_path="$(awk '$1 == "LD" && $2 == "=" { print $3; exit }' \
      "${web_build_dir}/Makefile")"
    [[ -x "$linker_path" ]] || fail "configured linker is not executable: $linker_path"
    printf 'host-tool-sha256 ld %s %s\n' \
      "$linker_path" "$(sha_digest 256 "$linker_path")"
    printf 'host-tool-version ld %s\n' "$("$linker_path" -v 2>&1 | sed -n '1p')"
    printf 'host-uname %s\n' "$(uname -a)"
    printf 'generated-clean-final-change-sha256 %s\n' \
      "$(sha_digest 256 "${out_dir}/clean-final.ch")"
    printf 'generated-instrumentable-final-change-sha256 %s\n' \
      "$(sha_digest 256 "${out_dir}/instrumentable-final.ch")"
    printf 'executable clean %s %s\n' \
      "${clean_executable#"${repo_root}/"}" "$(sha_digest 256 "$clean_executable")"
    printf 'executable instrumentable %s %s\n' \
      "${instrumentable_executable#"${repo_root}/"}" \
      "$(sha_digest 256 "$instrumentable_executable")"
    printf 'smoke-ordinary-log-sha256 clean %s\n' \
      "$(sha_digest 256 "${out_dir}/smoke/clean/ordinary.log")"
    printf 'smoke-ordinary-log-sha256 instrumentable %s\n' \
      "$(sha_digest 256 "${out_dir}/smoke/instrumentable/ordinary.log")"
    printf 'transition-trace-sha256 instrumentable %s\n' \
      "$(sha_digest 256 "${out_dir}/transitions/instrumentable/tex82-events.jsonl")"
    for variant in clean instrumentable instrumentable-repeat; do
      printf 'transition-terminal-sha256 %s %s\n' "$variant" \
        "$(sha_digest 256 "${out_dir}/transitions/${variant}/terminal.txt")"
      printf 'transition-ordinary-log-sha256 %s %s\n' "$variant" \
        "$(sha_digest 256 "${out_dir}/transitions/${variant}/ordinary.log")"
      printf 'transition-dvi-sha256 %s %s\n' "$variant" \
        "$(sha_digest 256 "${out_dir}/transitions/${variant}/transitions.dvi")"
      printf 'transition-effect-output-sha256 %s %s\n' "$variant" \
        "$(sha_digest 256 \
          "${out_dir}/transitions/${variant}/transitions-effects.out")"
    done
  } > "$record"
}

mkdir -p "$bin_dir"
fetch_source
extract_source
configure_tools
build_variant "$clean_executable"
cp "${web_build_dir}/tex-final.ch" "${out_dir}/clean-final.ch"
build_variant "$instrumentable_executable" "$instrumentation_change"
cp "${web_build_dir}/tex-final.ch" "${out_dir}/instrumentable-final.ch"
run_smoke "$clean_executable" clean
run_smoke "$instrumentable_executable" instrumentable
run_transitions "$clean_executable" clean
run_transitions "$instrumentable_executable" instrumentable
run_transitions "$instrumentable_executable" instrumentable-repeat
cmp "${out_dir}/smoke/clean/terminal.txt" \
  "${out_dir}/smoke/instrumentable/terminal.txt" >/dev/null ||
  fail "instrumentable oracle changed ordinary terminal output"
cmp "${out_dir}/smoke/clean/ordinary.log" \
  "${out_dir}/smoke/instrumentable/ordinary.log" >/dev/null ||
  fail "instrumentable oracle changed ordinary log output"
for channel in terminal.txt ordinary.log status.txt transitions.dvi \
  transitions-effects.out; do
  cmp "${out_dir}/transitions/clean/${channel}" \
    "${out_dir}/transitions/instrumentable/${channel}" >/dev/null ||
    fail "instrumentable oracle changed transition ${channel}"
  cmp "${out_dir}/transitions/instrumentable/${channel}" \
    "${out_dir}/transitions/instrumentable-repeat/${channel}" >/dev/null ||
    fail "repeated instrumentable oracle changed transition ${channel}"
done
cmp "${out_dir}/transitions/instrumentable/tex82-events.jsonl" \
  "${out_dir}/transitions/instrumentable-repeat/tex82-events.jsonl" >/dev/null ||
  fail "instrumentable oracle emitted a nondeterministic transition trace"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
  "${out_dir}/transitions/instrumentable/tex82-events.jsonl"
transition_trace="${out_dir}/transitions/instrumentable/tex82-events.jsonl"
while IFS='|' read -r family boundary fixture seam pattern extra; do
  [[ -z "$family" || "$family" == \#* ]] && continue
  [[ -n "$boundary" && -n "$fixture" && -n "$seam" && -n "$pattern" &&
    -z "${extra:-}" ]] ||
    fail "malformed semantic event matrix row for ${family:-unknown}"
  grep -Fq "$pattern" "$transition_trace" ||
    fail "transition trace is missing $family/$boundary from $fixture at $seam"
done < "$semantic_event_matrix"
write_build_record
printf '%s\n' "$bin_dir"
