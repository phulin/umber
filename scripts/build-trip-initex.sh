#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

manifest="tests/trip-reference-manifest.txt"
source_lock="tests/texlive-source.lock"
source_name="$(awk '$1 == "archive" { print $2 }' "$source_lock")"
cache_root="third_party/texlive-source"
source_tar="${cache_root}/${source_name}"
source_dir="${cache_root}/src"
build_dir="${cache_root}/build-trip-20260301"
target_dir="${CARGO_TARGET_DIR:-target}"
[[ "$target_dir" == /* ]] || target_dir="${repo_root}/${target_dir}"
out_dir="${target_dir}/trip-initex"
bin_dir="${out_dir}/bin"
trip_cflags="-O2"
trip_cxxflags="-O2"
offline=0

usage() {
  cat <<'EOF'
usage: scripts/build-trip-initex.sh [--offline]

Build the pinned TeX Live 2026 classic TeX and TeXware programs used for the
official TRIP reference phase. The archive and every relevant in-archive input
are hash-verified. After the first download, --offline performs no network I/O.

The resulting wrappers are under target/trip-initex/bin for manual reference
work and the future transcript-parity tier.
EOF
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --offline) offline=1 ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'build-trip-initex.sh: unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

fail() { printf 'build-trip-initex.sh: %s\n' "$*" >&2; exit 1; }

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
  local kind path expected extra actual
  while read -r kind path expected extra; do
    [[ -z "${kind:-}" || "$kind" == \#* || "$kind" == archive ]] && continue
    [[ "$kind" == sha256 && -z "${extra:-}" ]] || fail "malformed input pin: $kind $path $expected ${extra:-}"
    [[ -f "${source_dir}/${path}" ]] || fail "missing pinned source input ${source_dir}/${path}"
    actual="$(sha_digest 256 "${source_dir}/${path}")"
    [[ "$actual" == "$expected" ]] || fail "sha256 mismatch for $path: expected $expected, got $actual"
  done < "$manifest"
}

extract_source() {
  verify_inputs
}

build_tools() {
  mkdir -p "$build_dir"
  if [[ ! -f "${build_dir}/Makefile" ]]; then
    (
      cd "$build_dir"
      ../src/configure --without-x --disable-shared --disable-all-pkgs \
        --enable-tex --disable-synctex --disable-xetex --enable-missing -C \
        CFLAGS="$trip_cflags" CXXFLAGS="$trip_cxxflags"
    )
  fi
  # The top-level target configures the selected Web2C subtree and its small
  # dependency set; the second target names the only two programs retained.
  if [[ ! -f "${build_dir}/texk/web2c/Makefile" ]]; then
    make -C "$build_dir"
  fi
  make -C "${build_dir}/texk/web2c" tex dvitype
  local tool
  for tool in tex dvitype; do
    [[ -x "${build_dir}/texk/web2c/${tool}" ]] || fail "expected $tool was not built"
  done
}

write_wrappers() {
  mkdir -p "$bin_dir"
  local tool wrapper real
  for tool in tex dvitype; do
    wrapper="${bin_dir}/umber-trip-${tool}"
    real="${repo_root}/${build_dir}/texk/web2c/${tool}"
    {
      printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail'
      printf 'export TEXMFCNF=%q\n' "${repo_root}/${source_dir}/texk/web2c/triptrap"
      printf 'export LC_ALL=C\nexport LANGUAGE=C\n'
      printf 'exec %q "$@"\n' "$real"
    } > "$wrapper"
    chmod +x "$wrapper"
  done
  ln -sf umber-trip-tex "${bin_dir}/umber-trip-initex"
}

write_build_record() {
  local record="${out_dir}/build-record.txt" tool
  {
    printf 'archive-url %s\narchive-sha512 %s\n' "$archive_url" "$archive_sha512"
    printf 'configure ../src/configure --without-x --disable-shared --disable-all-pkgs --enable-tex --disable-synctex --disable-xetex --enable-missing -C CFLAGS=%q CXXFLAGS=%q\n' "$trip_cflags" "$trip_cxxflags"
    printf 'make make -C third_party/texlive-source/build && make -C third_party/texlive-source/build/texk/web2c tex dvitype\n'
    while read -r tool _; do
      [[ -n "${tool:-}" ]] && printf 'tool-sha256 %s %s\n' "$tool" "$(sha_digest 256 "${build_dir}/texk/web2c/${tool}")"
    done < <(printf '%s\n' 'tex x' 'dvitype x')
  } > "$record"
}

fetch_source
extract_source
build_tools
write_wrappers
write_build_record
printf '%s\n' "$bin_dir"
