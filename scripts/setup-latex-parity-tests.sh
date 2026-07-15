#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="${repo_root}/tests/latex-parity-manifest.txt"
parity_root="${repo_root}/third_party/latex2e-parity"
archive="${parity_root}/latex2e.tar.gz"
source_dir="${parity_root}/source"
case_list="${parity_root}/dvi-cases.txt"
offline=0

usage() {
  cat <<'EOF'
usage: scripts/setup-latex-parity-tests.sh [--offline]

Fetches, verifies, and extracts the pinned, unmodified LaTeX2e regression
suite used by scripts/check-latex-parity.sh. --offline only verifies an
already-downloaded archive and never accesses the network.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --offline) offline=1; shift ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'setup-latex-parity-tests.sh: unknown option: %s\n' "$1" >&2; exit 2 ;;
  esac
done

fail() {
  printf 'setup-latex-parity-tests.sh: %s\n' "$*" >&2
  exit 1
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

field() {
  awk -v key="$1" '$1 == key { print $2 }' "$manifest"
}

verify_file() {
  local path="$1"
  local expected_bytes="$2"
  local expected_hash="$3"
  [[ -f "$path" ]] || fail "missing pinned input: $path"
  local actual_bytes
  actual_bytes="$(wc -c < "$path" | tr -d ' ')"
  [[ "$actual_bytes" == "$expected_bytes" ]] || \
    fail "length mismatch for $path: expected $expected_bytes, got $actual_bytes"
  local actual_hash
  actual_hash="$(sha256 "$path")"
  [[ "$actual_hash" == "$expected_hash" ]] || \
    fail "hash mismatch for $path: expected $expected_hash, got $actual_hash"
}

[[ -f "$manifest" ]] || fail "missing manifest: $manifest"
archive_url="$(field archive_url)"
archive_bytes="$(field archive_bytes)"
archive_hash="$(field archive_sha256)"
archive_root="$(field archive_root)"
[[ -n "$archive_url" && -n "$archive_bytes" && -n "$archive_hash" && -n "$archive_root" ]] || \
  fail "manifest is missing archive metadata"

mkdir -p "$parity_root"
if [[ ! -f "$archive" ]]; then
  [[ $offline -eq 0 ]] || fail "archive unavailable in offline mode: $archive"
  tmp_archive="${archive}.tmp"
  rm -f "$tmp_archive"
  curl --fail --location --retry 3 --output "$tmp_archive" "$archive_url"
  verify_file "$tmp_archive" "$archive_bytes" "$archive_hash"
  mv "$tmp_archive" "$archive"
fi
verify_file "$archive" "$archive_bytes" "$archive_hash"

stamp="${source_dir}/.umber-latex2e-snapshot"
if [[ ! -f "$stamp" || "$(cat "$stamp")" != "$archive_hash" ]]; then
  rm -rf "$source_dir"
  mkdir -p "$source_dir"
  tar -xzf "$archive" -C "$source_dir" --strip-components=1
  printf '%s\n' "$archive_hash" > "$stamp"
fi

selection="$(field selection)"
expected_cases="$(field expected_cases)"
[[ "$selection" == standard-tlg-shipout ]] || fail "unsupported selection rule: $selection"
[[ "$expected_cases" =~ ^[1-9][0-9]*$ ]] || fail "invalid expected_cases value"

tmp_cases="${case_list}.tmp"
: > "$tmp_cases"
while read -r kind scope extra; do
  [[ "$kind" == scope ]] || continue
  [[ -z "${extra:-}" ]] || fail "invalid scope record: $scope"
  [[ "$scope" != /* && "$scope" != *..* ]] || fail "unsafe scope: $scope"
  scope_dir="${source_dir}/${scope}"
  [[ -d "$scope_dir" ]] || fail "missing pinned scope: $scope"
  while IFS= read -r lvt; do
    tlg="${lvt%.lvt}.tlg"
    if [[ -f "$tlg" ]] && grep -Eq 'Completed box being shipped out|\\box255' "$tlg"; then
      printf '%s\n' "${lvt#${source_dir}/}" >> "$tmp_cases"
    fi
  done < <(find "$scope_dir" -type f -name '*.lvt' -print)
done < "$manifest"
LC_ALL=C sort -u -o "$tmp_cases" "$tmp_cases"
actual_cases="$(wc -l < "$tmp_cases" | tr -d ' ')"
[[ "$actual_cases" == "$expected_cases" ]] || \
  fail "selection produced $actual_cases cases, expected $expected_cases"
mv "$tmp_cases" "$case_list"

printf 'LaTeX2e parity snapshot verified: %s (%s cases)\n' \
  "$(field snapshot)" "$actual_cases"
