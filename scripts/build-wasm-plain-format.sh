#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
lock_file="${repo_root}/crates/umber-wasm/assets/plain-source.lock"
output_dir="${repo_root}/crates/umber-wasm/assets"
texmf_dist="${UMBER_TEXMF_DIST:-/usr/local/texlive/2025/texmf-dist}"
check_only=0

usage() {
  cat <<'EOF'
usage: scripts/build-wasm-plain-format.sh [--texmf-dist PATH] [--output-dir PATH] [--check]

Builds the Umber-native Plain format from the exact TeX Live inputs recorded in
crates/umber-wasm/assets/plain-source.lock. UMBER_TEXMF_DIST may provide the
default pinned texmf-dist root. --check regenerates and compares committed
plain.fmt and plain-format.json without replacing them.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --texmf-dist)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --texmf-dist' >&2; exit 2; }
      texmf_dist="$2"
      shift 2
      ;;
    --output-dir)
      [[ $# -ge 2 ]] || { printf '%s\n' 'missing path after --output-dir' >&2; exit 2; }
      output_dir="$2"
      shift 2
      ;;
    --check)
      check_only=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      printf 'build-wasm-plain-format.sh: unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

fail() {
  printf 'build-wasm-plain-format.sh: %s\n' "$*" >&2
  exit 1
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

[[ -d "$texmf_dist" ]] || fail "missing pinned texmf-dist root: $texmf_dist"
[[ -f "$lock_file" ]] || fail "missing source lock: $lock_file"

distribution="$(awk '$1 == "distribution" { print $2 }' "$lock_file")"
format_schema="$(awk '$1 == "format_schema" { print $2 }' "$lock_file")"
source_date_epoch="$(awk '$1 == "source_date_epoch" { print $2 }' "$lock_file")"
[[ -n "$distribution" && -n "$format_schema" && -n "$source_date_epoch" ]] || \
  fail "source lock is missing required metadata"

tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

stage_sources() {
  local destination="$1"
  local kind staged relative expected extra
  mkdir -p "$destination"
  while read -r kind staged relative expected extra; do
    [[ -z "${kind:-}" || "$kind" == \#* ]] && continue
    [[ "$kind" != source ]] && continue
    [[ -z "${extra:-}" ]] || fail "invalid source lock entry for $staged"
    [[ "$relative" != /* && "$relative" != *..* && "$relative" != *\\* ]] || \
      fail "unsafe source path in lock: $relative"
    local source="${texmf_dist}/${relative}"
    [[ -f "$source" ]] || fail "missing pinned source: $source"
    local actual
    actual="$(sha256 "$source")"
    [[ "$actual" == "$expected" ]] || \
      fail "hash mismatch for $relative: expected $expected, got $actual"
    cp "$source" "${destination}/${staged}"
  done < "$lock_file"
  printf '\\input plain \\dump\n' > "${destination}/build.tex"
}

build_one() {
  local destination="$1"
  stage_sources "$destination"
  (
    cd "$destination"
    SOURCE_DATE_EPOCH="$source_date_epoch" "$umber_bin" run build.tex --format-out plain.fmt
  )
}

cd "$repo_root"
cargo build -p umber
umber_bin="${CARGO_TARGET_DIR:-${repo_root}/target}/debug/umber"
[[ -x "$umber_bin" ]] || fail "Umber binary was not built at $umber_bin"

build_one "${tmp_root}/first"
build_one "${tmp_root}/second"
cmp "${tmp_root}/first/plain.fmt" "${tmp_root}/second/plain.fmt" || \
  fail "two clean format generations were not byte-identical"

format_file="${tmp_root}/first/plain.fmt"
magic="$(od -An -t x1 -N 8 "$format_file" | tr -d ' \n')"
actual_schema="$(od -An -t u4 -j 8 -N 4 "$format_file" | tr -d ' \n')"
[[ "$magic" == 554d4252464d5400 ]] || fail "generated file lacks Umber format magic"
[[ "$actual_schema" == "$format_schema" ]] || \
  fail "generated schema $actual_schema does not match locked schema $format_schema"

cat > "${tmp_root}/first/document.tex" <<'EOF'
Plain format equivalence: $a^2+b^2=c^2$.par
\bye
EOF
printf '\\input plain \\input document\n' > "${tmp_root}/first/source-run.tex"
(
  cd "${tmp_root}/first"
  SOURCE_DATE_EPOCH="$source_date_epoch" "$umber_bin" run source-run.tex --dvi source.dvi
  SOURCE_DATE_EPOCH="$source_date_epoch" "$umber_bin" run document.tex --format plain.fmt --dvi loaded.dvi
)
cmp "${tmp_root}/first/source.dvi" "${tmp_root}/first/loaded.dvi" || \
  fail "source-initialized and format-loaded Plain DVI differ"

format_sha256="$(sha256 "$format_file")"
format_bytes="$(wc -c < "$format_file" | tr -d ' ')"
source_manifest_sha256="$(sha256 "$lock_file")"
package_id="$(cargo pkgid -p umber)"
engine_version="${package_id##*#}"

cat > "${tmp_root}/plain-format.json" <<EOF
{
  "schema": 1,
  "name": "plain",
  "object": "sha256-${format_sha256}",
  "sha256": "${format_sha256}",
  "bytes": ${format_bytes},
  "engine": "umber",
  "engineVersion": "${engine_version}",
  "formatSchema": ${format_schema},
  "sourceDistribution": "${distribution}",
  "sourceManifestSha256": "${source_manifest_sha256}",
  "sourceDateEpoch": ${source_date_epoch}
}
EOF

if [[ "$check_only" -eq 1 ]]; then
  cmp "$format_file" "${output_dir}/plain.fmt" || fail "committed plain.fmt is stale"
  cmp "${tmp_root}/plain-format.json" "${output_dir}/plain-format.json" || \
    fail "committed plain-format.json is stale"
else
  mkdir -p "$output_dir"
  cp "$format_file" "${output_dir}/plain.fmt"
  cp "${tmp_root}/plain-format.json" "${output_dir}/plain-format.json"
fi

printf 'Umber Plain format: sha256=%s bytes=%s schema=%s source=%s\n' \
  "$format_sha256" "$format_bytes" "$format_schema" "$distribution"
