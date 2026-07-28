#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

manifest="tests/etex26-oracle-manifest.txt"
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
out_dir="${target_dir}/etex26-oracle"
bin_dir="${out_dir}/bin"
instrumentation_change="${UMBER_ETEX26_INSTRUMENTATION_CHANGE:-${repo_root}/tests/etex26-oracle/instrumentation.ch}"
smoke_input="${repo_root}/tests/etex26-oracle/smoke.tex"
transition_input="${repo_root}/tests/etex26-oracle/transitions.tex"
transition_child="${repo_root}/tests/etex26-oracle/transitions-child.tex"
case_shift_input="${repo_root}/tests/etex26-oracle/case-shift.tex"
semantic_event_matrix="${repo_root}/tests/etex26-oracle/semantic-event-matrix.txt"
extension_input="${repo_root}/tests/etex26-oracle/extensions.tex"
extension_readline_input="${repo_root}/tests/etex26-oracle/extensions-readline.txt"
extension_event_matrix="${repo_root}/tests/etex26-oracle/extension-event-matrix.txt"
extension_primitive_audit="${repo_root}/tests/etex26-oracle/extension-primitive-audit.txt"
cflags="-O2"
cxxflags="-O2"
source_date_epoch="${SOURCE_DATE_EPOCH:-1783604160}"
offline=0
trip_profile_change="${out_dir}/trip-profile-instrumentation.ch"
trip_geometry_profile_change="${out_dir}/trip-geometry-profile-instrumentation.ch"

usage() {
  cat <<'EOF'
usage: scripts/build-etex26-oracle.sh [--offline]

Acquire and verify the pinned TeX Live 2025 source snapshot, then build
canonical e-TeX 2.6 clean and instrumented executables. Each build is
published under separately named compatibility and extended-mode profiles.
The profiles differ by the canonical INITEX invocation: extended mode consumes
a leading "*" before the input name.
The final repository change emits the shared schema-v1 base command trace and
focused extended-mode expansion/token-construction observations.

Outputs and a complete identity record are written under target/etex26-oracle.
After the first acquisition, --offline performs no network I/O.
EOF
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --offline) offline=1 ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'build-etex26-oracle: unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

fail() {
  printf 'build-etex26-oracle: %s\n' "$*" >&2
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
        --enable-etex --disable-synctex --disable-xetex --enable-missing -C \
        CFLAGS="$cflags" CXXFLAGS="$cxxflags"
    )
  fi
  if [[ ! -f "${web_build_dir}/Makefile" ]]; then
    make -C "$build_dir"
  fi
  make -C "$web_build_dir" tie tangle web2c/web2c
}

web_sources=(
  "${web_source_dir}/tex.web"
  "${web_source_dir}/etexdir/etex.ch"
)

upstream_changes=(
  "${web_source_dir}/etexdir/tex.ch0"
  "${web_source_dir}/tex.ch"
  "${web_source_dir}/zlib-fmt.ch"
  "${web_source_dir}/enctexdir/enctex1.ch"
  "${web_source_dir}/enctexdir/enctex-tex.ch"
  "${web_source_dir}/enctexdir/enctex2.ch"
  "${web_source_dir}/synctexdir/synctex-def.ch0"
  "${web_source_dir}/synctexdir/synctex-mem.ch0"
  "${web_source_dir}/synctexdir/synctex-e-mem.ch0"
  "${web_source_dir}/synctexdir/synctex-e-mem.ch1"
  "${web_source_dir}/synctexdir/synctex-rec.ch0"
  "${web_source_dir}/synctexdir/synctex-rec.ch1"
  "${web_source_dir}/synctexdir/synctex-e-rec.ch0"
  "${web_source_dir}/etexdir/tex.ch1"
  "${web_source_dir}/etexdir/tex.ech"
  "${web_source_dir}/tex-binpool.ch"
)

reset_etex_products() {
  rm -f \
    "${web_build_dir}/etex-final.ch" \
    "${web_build_dir}/etex-tangle" \
    "${web_build_dir}/etex.p" \
    "${web_build_dir}/etex.pool" \
    "${web_build_dir}/etex-web2c" \
    "${web_build_dir}/etexini.c" \
    "${web_build_dir}/etex0.c" \
    "${web_build_dir}/etexcoerce.h" \
    "${web_build_dir}/etexd.h" \
    "${web_build_dir}/etex-pool.c" \
    "${web_build_dir}/etex-etexini.o" \
    "${web_build_dir}/etex-etex0.o" \
    "${web_build_dir}/etex-etex-pool.o" \
    "${web_build_dir}/etexdir/etex-etexextra.o" \
    "${web_build_dir}/etex"
}

generate_canonical_web() {
  (
    cd "$web_build_dir"
    TEXMFCNF="${source_dir}/texk/kpathsea" WEBINPUTS=".:${web_source_dir}" \
      ./tie -m etex.web "${web_sources[@]}"
  )
  cp "${web_build_dir}/etex.web" "${out_dir}/canonical-etex.web"
}

build_variant() {
  local destination="$1"
  shift
  local change_stack=("${upstream_changes[@]}" "$@")
  reset_etex_products
  (
    cd "$web_build_dir"
    TEXMFCNF="${source_dir}/texk/kpathsea" WEBINPUTS=".:${web_source_dir}" \
      ./tie -c etex-final.ch etex.web "${change_stack[@]}"
    # Automake models etex-tangle as a non-materialized target. Materialize a
    # private timestamp barrier so the later compile/link step consumes the
    # exact final change tangled here instead of regenerating upstream etex.p.
    touch etex.ch etex-tangle
    TEXMFCNF="${source_dir}/texk/kpathsea" WEBINPUTS=".:${web_source_dir}" \
      ./tangle etex etex-final
    AM_V_P=false ./web2c-sh etex-web2c etex
  )
  make -C "$web_build_dir" etex
  [[ -x "${web_build_dir}/etex" ]] || fail "e-TeX executable was not built"
  cp "${web_build_dir}/etex" "$destination"
  chmod +x "$destination"
}

write_trip_profile_changes() {
  local filtered="${out_dir}/trip-profile-filtered.ch"
  sed \
    -e 's/umber_trip_profile:=false;/umber_trip_profile:=true;/' \
    -e 's/if not umber_trace_opened then return;/if (not umber_trace_opened)or(umber_trip_profile) then return;/' \
    -e 's/or(not umber_mutation_command)/or(umber_trip_profile)or(not umber_mutation_command)/' \
    "$instrumentation_change" >"$filtered"
  awk '
    /^procedure umber_trace_input\(/ { mode="input" }
    /^procedure umber_trace_effect\(/ { mode="effect" }
    /^procedure umber_trace_finish;/ { mode="finish" }
    $0 == "if (not umber_trace_opened)or(umber_trip_profile) then return;" {
      if (mode == "input") {
        print "if not umber_trace_opened then return;"
        print "if umber_trip_profile and (transition<>2) then return;"
        next
      }
      if (mode == "effect") {
        print "if not umber_trace_opened then return;"
        print "if umber_trip_profile and (effect_kind<>4) then return;"
        next
      }
      if (mode == "finish") {
        print "if not umber_trace_opened then return;"
        next
      }
    }
    { print }
    $0 == "umber_trace_begin;" { mode="" }
  ' "$filtered" >"$trip_profile_change"
  sed 's/umber_geometry_profile:=false;/umber_geometry_profile:=true;/' \
    "$trip_profile_change" >"$trip_geometry_profile_change"
  rm -f "$filtered"
}

profile_executable() {
  local build_profile="$1" engine_profile="$2"
  printf '%s/umber-etex26-%s-oracle-%s' \
    "$bin_dir" "$engine_profile" "$build_profile"
}

publish_variant() {
  local built="$1" build_profile="$2" engine_profile destination
  for engine_profile in compatibility extended; do
    destination="$(profile_executable "$build_profile" "$engine_profile")"
    cp "$built" "$destination"
    chmod +x "$destination"
  done
}

run_smoke() {
  local executable="$1" build_profile="$2" engine_profile="$3"
  local run_dir="${out_dir}/smoke/${build_profile}-${engine_profile}"
  local input_name="smoke.tex" status=0
  [[ "$engine_profile" == compatibility ]] || input_name="*smoke.tex"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$smoke_input" "${run_dir}/smoke.tex"
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/web2c/triptrap" \
      "$executable" -ini -interaction=batchmode "$input_name" \
      >terminal.txt 2>&1
  ) || status="$?"
  [[ "$status" -eq 0 ]] ||
    fail "$build_profile/$engine_profile smoke exited with status $status"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  [[ -f "${run_dir}/smoke.log" ]] ||
    fail "$build_profile/$engine_profile smoke did not write smoke.log"
  [[ -f "${run_dir}/smoke.dvi" ]] ||
    fail "$build_profile/$engine_profile smoke did not write smoke.dvi"
  grep -q 'e-TeX, Version 3.141592653-2.6' "${run_dir}/smoke.log" ||
    fail "$build_profile/$engine_profile did not identify canonical e-TeX 2.6"
  grep -q 'UMBER-ETEX26-ORACLE-SMOKE' "${run_dir}/smoke.log" ||
    fail "$build_profile/$engine_profile smoke marker is absent"
  grep -q 'COUNT=42' "${run_dir}/smoke.log" ||
    fail "$build_profile/$engine_profile arithmetic marker is absent"
  if [[ "$engine_profile" == compatibility ]]; then
    grep -q 'PROFILE=COMPATIBILITY' "${run_dir}/smoke.log" ||
      fail "$build_profile compatibility profile exposed e-TeX primitives"
    ! grep -q 'entering extended mode' "${run_dir}/smoke.log" ||
      fail "$build_profile compatibility profile entered extended mode"
  else
    grep -q 'PROFILE=EXTENDED' "${run_dir}/smoke.log" &&
      grep -q 'REVISION=.6' "${run_dir}/smoke.log" ||
      fail "$build_profile extended profile did not expose e-TeX primitives"
    grep -q 'entering extended mode' "${run_dir}/smoke.log" ||
      fail "$build_profile extended profile did not enter extended mode"
  fi
  sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/smoke.log" \
    >"${run_dir}/ordinary.log"
}

run_transitions() {
  local executable="$1" build_profile="$2" engine_profile="$3"
  local run_dir="${out_dir}/transitions/${build_profile}-${engine_profile}"
  local input_name="transitions.tex" status=0
  [[ "$engine_profile" == compatibility ]] || input_name="*transitions.tex"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$transition_input" "${run_dir}/transitions.tex"
  cp "$transition_child" "${run_dir}/transitions-child.tex"
  cp "$case_shift_input" "${run_dir}/case-shift.tex"
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/web2c/triptrap" \
      "$executable" -ini -interaction=nonstopmode "$input_name" \
      >terminal.txt 2>&1
  ) || status="$?"
  [[ "$status" -le 1 ]] ||
    fail "$build_profile/$engine_profile transition run exited with unexpected status $status"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  [[ -f "${run_dir}/transitions.log" ]] ||
    fail "$build_profile/$engine_profile transition run did not write transitions.log"
  [[ -f "${run_dir}/transitions.dvi" ]] ||
    fail "$build_profile/$engine_profile transition run did not write transitions.dvi"
  [[ -f "${run_dir}/transitions-effects.out" ]] ||
    fail "$build_profile/$engine_profile transition run did not write transition effects"
  grep -q 'UMBER-TEX82-TRANSITIONS' "${run_dir}/transitions.log" ||
    fail "$build_profile/$engine_profile transition marker is absent"
  sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/transitions.log" \
    >"${run_dir}/ordinary.log"
}

run_extensions() {
  local executable="$1" build_profile="$2" engine_profile="$3"
  local run_dir="${out_dir}/extensions/${build_profile}-${engine_profile}"
  local input_name="extensions.tex" status=0
  [[ "$engine_profile" == compatibility ]] || input_name="*extensions.tex"
  rm -rf "$run_dir"
  mkdir -p "$run_dir"
  cp "$extension_input" "${run_dir}/extensions.tex"
  cp "$extension_readline_input" "${run_dir}/extensions-readline.txt"
  (
    cd "$run_dir"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH="$source_date_epoch" FORCE_SOURCE_DATE=1 \
      TEXMFCNF="${source_dir}/texk/web2c/triptrap" \
      "$executable" -ini -interaction=batchmode "$input_name" \
      >terminal.txt 2>&1
  ) || status="$?"
  [[ "$status" -eq 0 ]] ||
    fail "$build_profile/$engine_profile extension run exited with status $status"
  printf '%s\n' "$status" >"${run_dir}/status.txt"
  [[ -f "${run_dir}/extensions.log" ]] ||
    fail "$build_profile/$engine_profile extension run did not write extensions.log"
  [[ -f "${run_dir}/extensions.dvi" ]] ||
    fail "$build_profile/$engine_profile extension run did not write extensions.dvi"
  [[ -f "${run_dir}/extensions-effects.out" ]] ||
    fail "$build_profile/$engine_profile extension run did not write extension effects"
  if [[ "$engine_profile" == compatibility ]]; then
    grep -q 'UMBER-ETEX26-EXTENSIONS-COMPATIBILITY' \
      "${run_dir}/extensions.log" ||
      fail "$build_profile compatibility extension exclusion marker is absent"
    ! grep -q 'UMBER-ETEX26-EXTENSIONS-EXTENDED' \
      "${run_dir}/extensions.log" ||
      fail "$build_profile compatibility profile executed e-TeX extensions"
  else
    grep -q 'UMBER-ETEX26-EXTENSIONS-EXTENDED' "${run_dir}/extensions.log" ||
      fail "$build_profile extended extension marker is absent"
    grep -Fq 'PROTECTED-COPY=A\protectedmacro B' \
      "${run_dir}/extensions.log" ||
      fail "$build_profile extended protected result is absent"
    grep -q 'UNEXPANDED-COPY=CORD' "${run_dir}/extensions.log" ||
      fail "$build_profile extended unexpanded result is absent"
    grep -q 'SCANTOKENS-BODY' "${run_dir}/extensions.log" &&
      grep -q 'EVERYEOF-REP' "${run_dir}/extensions.log" ||
      fail "$build_profile extended pseudo-file markers are absent"
  fi
  sed '1s/)  .*$/) <HOST-CLOCK>/' "${run_dir}/extensions.log" \
    >"${run_dir}/ordinary.log"
}

write_build_record() {
  local record="${out_dir}/build-record.txt"
  local linker_path path tool tool_path build_profile engine_profile executable
  {
    printf 'identity etex26-oracle-web2c-texlive-2025\n'
    printf 'engine e-TeX\nengine-version 2.6\ncharacter-profile eight-bit-exact\n'
    printf 'archive-url %s\narchive-sha512 %s\n' "$archive_url" "$archive_sha512"
    printf 'manifest-sha256 %s\n' "$(sha_digest 256 "$manifest")"
    printf 'source-date-epoch %s\n' "$source_date_epoch"
    printf 'configure ../src/configure --without-x --disable-shared --disable-all-pkgs --enable-etex --disable-synctex --disable-xetex --enable-missing -C CFLAGS=%q CXXFLAGS=%q\n' "$cflags" "$cxxflags"
    for path in "${web_sources[@]}"; do
      printf 'ordered-web-source-sha256 %s %s\n' \
        "${path#"${source_dir}/"}" "$(sha_digest 256 "$path")"
    done
    for path in "${upstream_changes[@]}"; do
      printf 'ordered-change-sha256 %s %s\n' \
        "${path#"${source_dir}/"}" "$(sha_digest 256 "$path")"
    done
    printf 'canonical-etex-web-sha256 %s\n' \
      "$(sha_digest 256 "${out_dir}/canonical-etex.web")"
    printf 'instrumentation-change %s\n' \
      "${instrumentation_change#"${repo_root}/"}"
    printf 'instrumentation-change-sha256 %s\n' \
      "$(sha_digest 256 "$instrumentation_change")"
    printf 'trip-profile-change-sha256 %s\n' \
      "$(sha_digest 256 "$trip_profile_change")"
    printf 'trip-geometry-profile-change-sha256 %s\n' \
      "$(sha_digest 256 "$trip_geometry_profile_change")"
    printf 'profile compatibility invocation-input smoke.tex\n'
    printf 'profile extended invocation-input *smoke.tex\n'
    printf 'tool-sha256 tie %s\n' "$(sha_digest 256 "${web_build_dir}/tie")"
    printf 'tool-sha256 tangle %s\n' "$(sha_digest 256 "${web_build_dir}/tangle")"
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
    linker_path="$(awk '$1 == "LD" && $2 == "=" { print $3; exit }' \
      "${web_build_dir}/Makefile")"
    [[ -x "$linker_path" ]] ||
      fail "configured linker is not executable: $linker_path"
    printf 'host-tool-sha256 ld %s %s\n' \
      "$linker_path" "$(sha_digest 256 "$linker_path")"
    printf 'host-tool-version ld %s\n' \
      "$("$linker_path" -v 2>&1 | sed -n '1p')"
    printf 'host-uname %s\n' "$(uname -a)"
    printf 'generated-clean-final-change-sha256 %s\n' \
      "$(sha_digest 256 "${out_dir}/clean-final.ch")"
    printf 'generated-instrumented-final-change-sha256 %s\n' \
      "$(sha_digest 256 "${out_dir}/instrumented-final.ch")"
    for build_profile in trip-profile trip-geometry-profile; do
      for engine_profile in compatibility extended; do
        executable="$(profile_executable "$build_profile" "$engine_profile")"
        printf 'executable %s %s %s %s\n' \
          "$build_profile" "$engine_profile" \
          "${executable#"${repo_root}/"}" "$(sha_digest 256 "$executable")"
      done
    done
    for build_profile in clean instrumented; do
      for engine_profile in compatibility extended; do
        executable="$(profile_executable "$build_profile" "$engine_profile")"
        printf 'executable %s %s %s %s\n' \
          "$build_profile" "$engine_profile" \
          "${executable#"${repo_root}/"}" "$(sha_digest 256 "$executable")"
        printf 'smoke-ordinary-log-sha256 %s %s %s\n' \
          "$build_profile" "$engine_profile" \
          "$(sha_digest 256 "${out_dir}/smoke/${build_profile}-${engine_profile}/ordinary.log")"
        printf 'smoke-dvi-sha256 %s %s %s\n' \
          "$build_profile" "$engine_profile" \
          "$(sha_digest 256 "${out_dir}/smoke/${build_profile}-${engine_profile}/smoke.dvi")"
        printf 'transition-terminal-sha256 %s %s %s\n' \
          "$build_profile" "$engine_profile" \
          "$(sha_digest 256 "${out_dir}/transitions/${build_profile}-${engine_profile}/terminal.txt")"
        printf 'transition-ordinary-log-sha256 %s %s %s\n' \
          "$build_profile" "$engine_profile" \
          "$(sha_digest 256 "${out_dir}/transitions/${build_profile}-${engine_profile}/ordinary.log")"
        printf 'transition-dvi-sha256 %s %s %s\n' \
          "$build_profile" "$engine_profile" \
          "$(sha_digest 256 "${out_dir}/transitions/${build_profile}-${engine_profile}/transitions.dvi")"
        printf 'transition-effect-output-sha256 %s %s %s\n' \
          "$build_profile" "$engine_profile" \
          "$(sha_digest 256 "${out_dir}/transitions/${build_profile}-${engine_profile}/transitions-effects.out")"
        printf 'extension-terminal-sha256 %s %s %s\n' \
          "$build_profile" "$engine_profile" \
          "$(sha_digest 256 "${out_dir}/extensions/${build_profile}-${engine_profile}/terminal.txt")"
        printf 'extension-ordinary-log-sha256 %s %s %s\n' \
          "$build_profile" "$engine_profile" \
          "$(sha_digest 256 "${out_dir}/extensions/${build_profile}-${engine_profile}/ordinary.log")"
        printf 'extension-dvi-sha256 %s %s %s\n' \
          "$build_profile" "$engine_profile" \
          "$(sha_digest 256 "${out_dir}/extensions/${build_profile}-${engine_profile}/extensions.dvi")"
        printf 'extension-effect-output-sha256 %s %s %s\n' \
          "$build_profile" "$engine_profile" \
          "$(sha_digest 256 "${out_dir}/extensions/${build_profile}-${engine_profile}/extensions-effects.out")"
        if [[ "$build_profile" == instrumented ]]; then
          printf 'transition-trace-sha256 %s %s %s\n' \
            "$build_profile" "$engine_profile" \
            "$(sha_digest 256 "${out_dir}/transitions/${build_profile}-${engine_profile}/etex26-events.jsonl")"
          printf 'extension-trace-sha256 %s %s %s\n' \
            "$build_profile" "$engine_profile" \
            "$(sha_digest 256 "${out_dir}/extensions/${build_profile}-${engine_profile}/etex26-events.jsonl")"
        fi
      done
    done
  } > "$record"
}

mkdir -p "$bin_dir"
fetch_source
extract_source
configure_tools
generate_canonical_web
write_trip_profile_changes
build_variant "${out_dir}/etex-clean"
cp "${web_build_dir}/etex-final.ch" "${out_dir}/clean-final.ch"
publish_variant "${out_dir}/etex-clean" clean
build_variant "${out_dir}/etex-instrumented" "$instrumentation_change"
cp "${web_build_dir}/etex-final.ch" "${out_dir}/instrumented-final.ch"
publish_variant "${out_dir}/etex-instrumented" instrumented
build_variant "${out_dir}/etex-trip-profile" "$trip_profile_change"
publish_variant "${out_dir}/etex-trip-profile" trip-profile
build_variant "${out_dir}/etex-trip-geometry-profile" "$trip_geometry_profile_change"
publish_variant "${out_dir}/etex-trip-geometry-profile" trip-geometry-profile
for build_profile in clean instrumented; do
  for engine_profile in compatibility extended; do
    run_smoke "$(profile_executable "$build_profile" "$engine_profile")" \
      "$build_profile" "$engine_profile"
    run_transitions "$(profile_executable "$build_profile" "$engine_profile")" \
      "$build_profile" "$engine_profile"
    run_extensions "$(profile_executable "$build_profile" "$engine_profile")" \
      "$build_profile" "$engine_profile"
  done
done

audit_extension_primitives() {
  local source_inventory audit_inventory primitive owner gate seam extra
  source_inventory="$(mktemp)"
  audit_inventory="$(mktemp)"
  awk 'match($0,/primitive\("[^"]+"/) {
    print substr($0,RSTART+11,RLENGTH-12)
  }' "${web_source_dir}/etexdir/etex.ch" | LC_ALL=C sort -u >"$source_inventory"
  while IFS='|' read -r primitive owner gate seam extra; do
    [[ -z "$primitive" || "$primitive" == \#* ]] && continue
    [[ -n "$owner" && -n "$gate" && -n "$seam" && -z "${extra:-}" ]] ||
      fail "malformed extension primitive audit row for ${primitive:-unknown}"
    [[ "$owner" == command-core || "$owner" == executor ]] ||
      fail "unknown extension primitive owner for $primitive: $owner"
    if [[ "$owner" == command-core ]]; then
      awk -F'|' -v gate="$gate" '$2 == gate { found=1 } END { exit !found }' \
        "$extension_event_matrix" ||
        fail "command-core primitive $primitive has no extension matrix boundary: $gate"
    fi
    printf '%s\n' "$primitive"
  done <"$extension_primitive_audit" | LC_ALL=C sort -u >"$audit_inventory"
  cmp "$source_inventory" "$audit_inventory" >/dev/null ||
    fail "extension primitive audit does not exactly cover canonical etex.ch primitives"
  rm -f "$source_inventory" "$audit_inventory"
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

audit_extension_primitives
for build_profile in clean instrumented; do
  cmp "$(profile_executable "$build_profile" compatibility)" \
    "$(profile_executable "$build_profile" extended)" >/dev/null ||
    fail "$build_profile profile aliases do not identify one canonical executable"
done
for engine_profile in compatibility extended; do
  compare_channels "instrumented $engine_profile smoke oracle" \
    "${out_dir}/smoke/clean-${engine_profile}" \
    "${out_dir}/smoke/instrumented-${engine_profile}" \
    terminal.txt ordinary.log status.txt smoke.dvi
  compare_channels "instrumented $engine_profile transition oracle" \
    "${out_dir}/transitions/clean-${engine_profile}" \
    "${out_dir}/transitions/instrumented-${engine_profile}" \
    terminal.txt ordinary.log status.txt transitions.dvi transitions-effects.out
  compare_channels "instrumented $engine_profile extension oracle" \
    "${out_dir}/extensions/clean-${engine_profile}" \
    "${out_dir}/extensions/instrumented-${engine_profile}" \
    terminal.txt ordinary.log status.txt extensions.dvi extensions-effects.out
done
for engine_profile in compatibility extended; do
  trace="${out_dir}/transitions/instrumented-${engine_profile}/etex26-events.jsonl"
  cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$trace"
  while IFS='|' read -r family boundary fixture seam pattern extra; do
    [[ -z "$family" || "$family" == \#* ]] && continue
    [[ -n "$boundary" && -n "$fixture" && -n "$seam" && -n "$pattern" &&
      -z "${extra:-}" ]] ||
      fail "malformed semantic event matrix row for ${family:-unknown}"
    grep -Fq "$pattern" "$trace" ||
      fail "$engine_profile trace is missing $family/$boundary from $fixture at $seam"
  done < "$semantic_event_matrix"
done
extended_trace="${out_dir}/extensions/instrumented-extended/etex26-events.jsonl"
compatibility_trace="${out_dir}/extensions/instrumented-compatibility/etex26-events.jsonl"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$extended_trace"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- "$compatibility_trace"
while IFS='|' read -r family boundary fixture seam pattern compatibility extra; do
  [[ -z "$family" || "$family" == \#* ]] && continue
  [[ -n "$boundary" && -n "$fixture" && -n "$seam" && -n "$pattern" &&
    ( "$compatibility" == absent || "$compatibility" == shared ) &&
    -z "${extra:-}" ]] ||
    fail "malformed extension event matrix row for ${family:-unknown}"
  grep -Fq "$pattern" "$extended_trace" ||
    fail "extended trace is missing $family/$boundary from $fixture at $seam"
  if [[ "$compatibility" == absent ]]; then
    ! grep -Fq "$pattern" "$compatibility_trace" ||
      fail "compatibility trace unexpectedly contains $family/$boundary"
  fi
done < "$extension_event_matrix"
run_transitions "$(profile_executable instrumented compatibility)" \
  instrumented-repeat compatibility
run_transitions "$(profile_executable instrumented extended)" \
  instrumented-repeat extended
run_extensions "$(profile_executable instrumented compatibility)" \
  instrumented-repeat compatibility
run_extensions "$(profile_executable instrumented extended)" \
  instrumented-repeat extended
for engine_profile in compatibility extended; do
  compare_channels "repeated instrumented $engine_profile transition oracle" \
    "${out_dir}/transitions/instrumented-${engine_profile}" \
    "${out_dir}/transitions/instrumented-repeat-${engine_profile}" \
    terminal.txt ordinary.log status.txt transitions.dvi transitions-effects.out \
    etex26-events.jsonl
  compare_channels "repeated instrumented $engine_profile extension oracle" \
    "${out_dir}/extensions/instrumented-${engine_profile}" \
    "${out_dir}/extensions/instrumented-repeat-${engine_profile}" \
    terminal.txt ordinary.log status.txt extensions.dvi extensions-effects.out \
    etex26-events.jsonl
done
write_build_record
printf '%s\n' "$bin_dir"
