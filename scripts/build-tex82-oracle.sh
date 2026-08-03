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
trip_profile_executable="${bin_dir}/umber-tex82-oracle-trip-profile"
trip_geometry_profile_executable="${bin_dir}/umber-tex82-oracle-trip-geometry-profile"
geometry_profile_executable="${bin_dir}/umber-tex82-oracle-geometry-profile"
instrumentation_change="${UMBER_TEX82_INSTRUMENTATION_CHANGE:-${repo_root}/tests/tex82-oracle/instrumentation.ch}"
smoke_input="${repo_root}/tests/tex82-oracle/smoke.tex"
geometry_input="${repo_root}/tests/tex82-oracle/geometry.tex"
geometry_expected="${repo_root}/tests/tex82-oracle/geometry-expected.jsonl"
clock_change="${repo_root}/tests/tex82-oracle/deterministic-clock.ch"
clock_input="${repo_root}/tests/tex82-oracle/clock.tex"
tex82_fixture_source_dir="${repo_root}/tests/tex82-oracle"
# One committed one-or-few-source minifixture per row: NAME:ENTRY-FILE:COMPANION-FILES
# (comma separated, empty when the entry file is fully self-contained). Sources
# that are inherently a multi-file interaction -- a source pushed by \input, an
# EOF observed mid-nested-file -- keep their companion in the same row instead
# of being forced apart; everything else runs as its own one-source job. Adding
# a row here and its sibling entry in tests/oracle-regeneration-manifest.txt is
# the whole mechanism for registering another split fixture: nothing else in
# this script names a fixture literally.
tex82_fixtures=(
  "command-transitions-v1:transitions.tex:input-recovery.tex,transitions-child.tex,input-eof-normal.tex,input-eof-defining.tex,input-eof-matching.tex,input-eof-absorbing.tex,input-eof-aligning.tex"
  "case-shift-v1:case-shift.tex:"
  "expansion-macros-v1:expansion-macros.tex:"
  "alignment-delivery-v1:alignment-delivery.tex:"
  "off-save-v1:off-save.tex:"
  "scanner-conditionals-v1:scanner-conditionals.tex:scanner-conditionals-eof.tex"
)
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
The geometry-profile executable is a separately selected schema-v3 observer;
it never changes the schema-v1 instrumentable executable or its trace.
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
  [[ -f "$instrumentation_change" ]] ||
    fail "missing instrumentation change file: $instrumentation_change"
  local fixture_spec fixture_name fixture_entry fixture_companions
  for fixture_spec in "${tex82_fixtures[@]}"; do
    IFS=':' read -r fixture_name fixture_entry fixture_companions <<<"$fixture_spec"
    [[ -f "${tex82_fixture_source_dir}/${fixture_entry}" ]] ||
      fail "missing ${fixture_name} entry source: ${fixture_entry}"
    [[ -f "${repo_root}/tests/tex82-oracle/${fixture_name}-semantic-matrix.txt" ]] ||
      fail "missing semantic matrix for ${fixture_name}"
  done
  [[ -f "$geometry_expected" ]] ||
    fail "missing geometry expectation: $geometry_expected"
}

write_trip_profile_change() {
  trip_profile_change="${out_dir}/trip-profile-instrumentation.ch"
  sed 's/umber_trip_profile:=false;/umber_trip_profile:=true;/' \
    "$instrumentation_change" >"$trip_profile_change"
}

write_geometry_profile_change() {
  geometry_profile_change="${out_dir}/geometry-profile-instrumentation.ch"
  sed 's/umber_geometry_profile:=false;/umber_geometry_profile:=true;/' \
    "$instrumentation_change" >"$geometry_profile_change"
}

write_trip_geometry_profile_change() {
  trip_geometry_profile_change="${out_dir}/trip-geometry-profile-instrumentation.ch"
  sed \
    -e 's/umber_trip_profile:=false;/umber_trip_profile:=true;/' \
    -e 's/umber_geometry_profile:=false;/umber_geometry_profile:=true;/' \
    "$instrumentation_change" >"$trip_geometry_profile_change"
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
oracle_base_changes=(
  "${upstream_changes[@]}"
  "$clock_change"
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
  local change_stack=("${oracle_base_changes[@]}" "$@")
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

run_clock() {
  local executable="$1" variant="$2" run_dir
  run_dir="${out_dir}/clock/${variant}"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$clock_input" "${run_dir}/clock.tex"
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      TEXMFCNF="${source_dir}/texk/web2c/triptrap" \
      "$executable" -ini -interaction=batchmode clock.tex >terminal.txt 2>&1
  )
  grep -Fq 'UMBER-TEX82-CLOCK: TIME=816,DAY=9,MONTH=7,YEAR=2026' \
    "${run_dir}/clock.log" ||
    fail "$variant did not observe the pinned TeX82 section 1337 clock"
  if [[ -f "${run_dir}/tex82-events.jsonl" ]]; then
    grep -Fq '"scanner":"internal","result":{"type":"integer","value":9}' \
      "${run_dir}/tex82-events.jsonl" ||
      fail "$variant did not emit the pinned day through the internal scanner"
    cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
      "${run_dir}/tex82-events.jsonl"
  fi
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

# Runs one committed tex82 command-transitions minifixture (see $tex82_fixtures
# above) under one built executable variant, and normalizes its log. Per-source
# content markers are deliberately not asserted here: the semantic-matrix
# pattern check driven from $tex82_fixtures after every variant has run is the
# precise version of that same sanity check, one committed JSON fragment per
# required command-core seam instead of a human log string.
run_tex82_fixture() {
  local name="$1" entry="$2" companions="$3" executable="$4" variant="$5"
  local run_dir status=0 stem companion
  run_dir="${out_dir}/${name}/${variant}"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "${tex82_fixture_source_dir}/${entry}" "${run_dir}/"
  if [[ -n "$companions" ]]; then
    local companion_list=()
    IFS=',' read -r -a companion_list <<<"$companions"
    for companion in "${companion_list[@]}"; do
      cp "${tex82_fixture_source_dir}/${companion}" "${run_dir}/"
    done
  fi
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/web2c/triptrap" \
      "$executable" -ini -interaction=nonstopmode "$entry" \
      >terminal.txt 2>&1
  ) || status="$?"
  [[ "$status" -le 1 ]] ||
    fail "${name}/${variant} run exited with unexpected status $status"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  stem="${entry%.tex}"
  [[ -f "${run_dir}/${stem}.log" ]] ||
    fail "${name}/${variant} run did not write ${stem}.log"
  sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/${stem}.log" \
    >"${run_dir}/ordinary.log"
}

# Runs one fixture's clean/instrumentable/instrumentable-repeat variants,
# checks that instrumentation left every observable channel unchanged and that
# the instrumented trace is deterministic, then requires every one of that
# fixture's committed semantic-matrix rows to be present in the trace.
run_and_validate_tex82_fixture() {
  local name="$1" entry="$2" companions="$3"
  local fixture_matrix="${repo_root}/tests/tex82-oracle/${name}-semantic-matrix.txt"
  local channel_path channel trace family boundary fixture seam pattern extra
  run_tex82_fixture "$name" "$entry" "$companions" "$clean_executable" clean
  run_tex82_fixture "$name" "$entry" "$companions" "$instrumentable_executable" \
    instrumentable
  run_tex82_fixture "$name" "$entry" "$companions" "$instrumentable_executable" \
    instrumentable-repeat
  for channel_path in "${out_dir}/${name}/clean"/*; do
    channel="$(basename "$channel_path")"
    case "$channel" in
      *.tex | *.log | tex82-events.jsonl) continue ;;
    esac
    cmp "${out_dir}/${name}/clean/${channel}" \
      "${out_dir}/${name}/instrumentable/${channel}" >/dev/null ||
      fail "instrumentable oracle changed ${name} ${channel}"
    cmp "${out_dir}/${name}/instrumentable/${channel}" \
      "${out_dir}/${name}/instrumentable-repeat/${channel}" >/dev/null ||
      fail "repeated instrumentable oracle changed ${name} ${channel}"
  done
  trace="${out_dir}/${name}/instrumentable/tex82-events.jsonl"
  cmp "$trace" "${out_dir}/${name}/instrumentable-repeat/tex82-events.jsonl" \
    >/dev/null ||
    fail "instrumentable oracle emitted a nondeterministic ${name} trace"
  cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$trace"
  while IFS='|' read -r family boundary fixture seam pattern extra; do
    [[ -z "$family" || "$family" == \#* ]] && continue
    [[ -n "$boundary" && -n "$fixture" && -n "$seam" && -n "$pattern" &&
      -z "${extra:-}" ]] ||
      fail "malformed semantic event matrix row for ${family:-unknown} in ${name}"
    grep -Fq "$pattern" "$trace" ||
      fail "${name} trace is missing $family/$boundary from $fixture at $seam"
  done < "$fixture_matrix"
}

run_geometry() {
  local executable="$1" variant="$2" run_dir
  run_dir="${out_dir}/geometry/${variant}"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$geometry_input" "${run_dir}/geometry.tex"
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/web2c/triptrap" \
      "$executable" -ini -interaction=batchmode geometry.tex >terminal.txt 2>&1
  )
  [[ -f "${run_dir}/geometry.log" ]] || fail "$variant geometry run did not write geometry.log"
  [[ -f "${run_dir}/geometry.dvi" ]] || fail "$variant geometry run did not write geometry.dvi"
  [[ -f "${run_dir}/tex82-events.jsonl" ]] || fail "$variant geometry run did not write a trace"
  grep -Fq 'UMBER-TEX82-GEOMETRY' "${run_dir}/geometry.log" ||
    fail "$variant geometry marker is absent"
}

validate_geometry_fixture() {
  local trace="$1" projected
  projected="$(mktemp)"
  {
    sed -n '1p' "$trace"
    awk '
      /"event":"geometry"/ {
        sub(/"sequence":[0-9]+/, "\"sequence\":" (sequence + 0))
        print
        sequence++
      }
    ' "$trace"
  } >"$projected"
  cmp "$geometry_expected" "$projected" >/dev/null || {
    rm -f "$projected"
    fail "geometry-profile oracle disagrees with the committed schema-v3 fixture"
  }
  cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$projected" ||
    fail "committed geometry projection is not a valid schema-v3 stream"
  rm -f "$projected"
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
    printf 'system-adapter-change %s\n' "${clock_change#"${repo_root}/"}"
    printf 'system-adapter-change-sha256 %s\n' "$(sha_digest 256 "$clock_change")"
    printf 'instrumentation-change %s\n' "${instrumentation_change#"${repo_root}/"}"
    printf 'instrumentation-change-sha256 %s\n' "$(sha_digest 256 "$instrumentation_change")"
    printf 'trip-profile-change-sha256 %s\n' "$(sha_digest 256 "$trip_profile_change")"
    printf 'geometry-profile-change-sha256 %s\n' "$(sha_digest 256 "$geometry_profile_change")"
    printf 'trip-geometry-profile-change-sha256 %s\n' \
      "$(sha_digest 256 "$trip_geometry_profile_change")"
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
    printf 'executable trip-profile %s %s\n' \
      "${trip_profile_executable#"${repo_root}/"}" \
      "$(sha_digest 256 "$trip_profile_executable")"
    printf 'executable geometry-profile %s %s\n' \
      "${geometry_profile_executable#"${repo_root}/"}" \
      "$(sha_digest 256 "$geometry_profile_executable")"
    printf 'executable trip-geometry-profile %s %s\n' \
      "${trip_geometry_profile_executable#"${repo_root}/"}" \
      "$(sha_digest 256 "$trip_geometry_profile_executable")"
    printf 'smoke-ordinary-log-sha256 clean %s\n' \
      "$(sha_digest 256 "${out_dir}/smoke/clean/ordinary.log")"
    printf 'smoke-ordinary-log-sha256 instrumentable %s\n' \
      "$(sha_digest 256 "${out_dir}/smoke/instrumentable/ordinary.log")"
    printf 'geometry-trace-sha256 geometry-profile %s\n' \
      "$(sha_digest 256 "${out_dir}/geometry/geometry-profile/tex82-events.jsonl")"
    local fixture_spec fixture_name fixture_entry fixture_companions variant
    for fixture_spec in "${tex82_fixtures[@]}"; do
      IFS=':' read -r fixture_name fixture_entry fixture_companions <<<"$fixture_spec"
      printf 'fixture-trace-sha256 %s instrumentable %s\n' "$fixture_name" \
        "$(sha_digest 256 \
          "${out_dir}/${fixture_name}/instrumentable/tex82-events.jsonl")"
      for variant in clean instrumentable instrumentable-repeat; do
        printf 'fixture-terminal-sha256 %s %s %s\n' "$fixture_name" "$variant" \
          "$(sha_digest 256 "${out_dir}/${fixture_name}/${variant}/terminal.txt")"
        printf 'fixture-ordinary-log-sha256 %s %s %s\n' "$fixture_name" "$variant" \
          "$(sha_digest 256 "${out_dir}/${fixture_name}/${variant}/ordinary.log")"
      done
    done
  } > "$record"
}

mkdir -p "$bin_dir"
fetch_source
extract_source
configure_tools
write_trip_profile_change
write_geometry_profile_change
write_trip_geometry_profile_change
build_variant "$clean_executable"
cp "${web_build_dir}/tex-final.ch" "${out_dir}/clean-final.ch"
build_variant "$instrumentable_executable" "$instrumentation_change"
cp "${web_build_dir}/tex-final.ch" "${out_dir}/instrumentable-final.ch"
run_smoke "$clean_executable" clean
run_smoke "$instrumentable_executable" instrumentable
run_clock "$clean_executable" clean
run_clock "$instrumentable_executable" instrumentable
run_clock "$instrumentable_executable" instrumentable-repeat
cmp "${out_dir}/clock/clean/terminal.txt" \
  "${out_dir}/clock/instrumentable/terminal.txt" >/dev/null ||
  fail "instrumentable oracle changed pinned-clock terminal output"
cmp "${out_dir}/clock/instrumentable/terminal.txt" \
  "${out_dir}/clock/instrumentable-repeat/terminal.txt" >/dev/null ||
  fail "repeated instrumentable oracle changed pinned-clock terminal output"
cmp "${out_dir}/clock/clean/clock.log" \
  "${out_dir}/clock/instrumentable/clock.log" >/dev/null ||
  fail "instrumentable oracle changed pinned-clock log output"
cmp "${out_dir}/clock/instrumentable/clock.log" \
  "${out_dir}/clock/instrumentable-repeat/clock.log" >/dev/null ||
  fail "repeated instrumentable oracle changed pinned-clock log output"
cmp "${out_dir}/smoke/clean/terminal.txt" \
  "${out_dir}/smoke/instrumentable/terminal.txt" >/dev/null ||
  fail "instrumentable oracle changed ordinary terminal output"
cmp "${out_dir}/smoke/clean/ordinary.log" \
  "${out_dir}/smoke/instrumentable/ordinary.log" >/dev/null ||
  fail "instrumentable oracle changed ordinary log output"
for fixture_spec in "${tex82_fixtures[@]}"; do
  IFS=':' read -r fixture_name fixture_entry fixture_companions <<<"$fixture_spec"
  run_and_validate_tex82_fixture "$fixture_name" "$fixture_entry" \
    "$fixture_companions"
done
build_variant "$trip_profile_executable" "$trip_profile_change"
build_variant "$trip_geometry_profile_executable" "$trip_geometry_profile_change"
build_variant "$geometry_profile_executable" "$geometry_profile_change"
run_geometry "$geometry_profile_executable" geometry-profile
run_geometry "$geometry_profile_executable" geometry-profile-repeat
cmp "${out_dir}/geometry/geometry-profile/tex82-events.jsonl" \
  "${out_dir}/geometry/geometry-profile-repeat/tex82-events.jsonl" >/dev/null ||
  fail "geometry-profile oracle emitted a nondeterministic trace"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
  "${out_dir}/geometry/geometry-profile/tex82-events.jsonl"
validate_geometry_fixture \
  "${out_dir}/geometry/geometry-profile/tex82-events.jsonl"
write_build_record
printf '%s\n' "$bin_dir"
