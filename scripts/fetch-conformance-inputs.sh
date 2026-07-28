#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

trip_manifest="tests/trip-manifest.txt"
trip_dir="third_party/trip"
offline=0
curl_bin="${CURL:-curl}"

usage() {
  cat <<'EOF'
usage: scripts/fetch-conformance-inputs.sh [--offline]

Acquire the external hyphenation, Computer Modern font, and pinned TRIP/e-TRIP
inputs used by the end-to-end conformance tests. With --offline, verify the
existing TRIP cache without performing network I/O.
EOF
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --offline) offline=1 ;;
    --help|-h) usage; exit 0 ;;
    *)
      printf 'fetch-conformance-inputs.sh: unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

fail() {
  printf 'fetch-conformance-inputs.sh: %s\n' "$*" >&2
  exit 1
}

if ! command -v kpsewhich >/dev/null 2>&1; then
  fail "could not locate kpsewhich; install TeX Live or MacTeX"
fi

hyphen_source="$(kpsewhich hyphen.tex || true)"
[[ -n "$hyphen_source" ]] || fail "kpsewhich could not locate hyphen.tex"
mkdir -p third_party/hyphen
if [[ -f third_party/hyphen/hyphen.tex ]] && \
    cmp -s "$hyphen_source" third_party/hyphen/hyphen.tex; then
  printf '%s already up to date\n' third_party/hyphen/hyphen.tex
else
  cp "$hyphen_source" third_party/hyphen/hyphen.tex
  printf 'fetched %s from %s\n' third_party/hyphen/hyphen.tex "$hyphen_source"
fi

fonts=(
  cmbsy10 cmbx10 cmbx5 cmbx6 cmbx7 cmbx8 cmbx9 cmcsc10 cmdunh10
  cmex10 cmmi10 cmmi5 cmmi6 cmmi7 cmmi8 cmmi9 cmmib10
  cmr10 cmr5 cmr6 cmr7 cmr8 cmr9
  cmsl10 cmsl8 cmsl9 cmsltt10 cmss10 cmssbx10 cmssi10 cmssq8 cmssqi8
  cmsy10 cmsy5 cmsy6 cmsy7 cmsy8 cmsy9
  cmti10 cmti7 cmti8 cmti9 cmtt10 cmtt8 cmtt9 cmu10 manfnt
)
mkdir -p third_party/fonts
missing_fonts=()
for font in "${fonts[@]}"; do
  source_path="$(kpsewhich "${font}.tfm" || true)"
  if [[ -z "$source_path" ]]; then
    missing_fonts+=("${font}.tfm")
    continue
  fi

  dest_path="third_party/fonts/${font}.tfm"
  if [[ -f "$dest_path" ]] && cmp -s "$source_path" "$dest_path"; then
    printf '%s already up to date\n' "$dest_path"
  else
    cp "$source_path" "$dest_path"
    printf 'fetched %s from %s\n' "$dest_path" "$source_path"
  fi
done
if (( ${#missing_fonts[@]} > 0 )); then
  fail "kpsewhich could not locate required TFM files: ${missing_fonts[*]}"
fi

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    fail "need shasum or sha256sum on PATH"
  fi
}

verify_hash() {
  local path="$1"
  local expected="$2"
  local name="$3"
  local actual
  actual="$(sha256_file "$path")"
  if [[ "$actual" != "$expected" ]]; then
    fail "SHA-256 mismatch for $name at $path: expected $expected, got $actual"
  fi
}

mkdir -p "$trip_dir"
while read -r -a fields; do
  name="${fields[0]:-}"
  [[ -z "${name:-}" || "$name" == \#* ]] && continue
  (( ${#fields[@]} >= 3 )) || fail "malformed manifest line for $name: expected file SHA-256 and at least one URL"
  expected="${fields[1]}"
  urls=("${fields[@]:2}")
  [[ "$expected" =~ ^[[:xdigit:]]{64}$ ]] ||
    fail "invalid SHA-256 for $name: $expected"
  seen_urls=" "
  for url in "${urls[@]}"; do
    [[ "$url" == https://* || "$url" == http://127.0.0.1:* || "$url" == http://localhost:* ]] ||
      fail "unsafe URL for $name: $url"
    [[ "$seen_urls" != *" $url "* ]] || fail "duplicate URL for $name: $url"
    seen_urls+="$url "
  done

  path="${trip_dir}/${name}"
  if [[ -f "$path" ]]; then
    verify_hash "$path" "$expected" "$name"
    printf 'verified %s\n' "$name"
    continue
  fi

  [[ "$offline" -eq 0 ]] || fail "missing $path while running --offline"
  tmp="${path}.tmp"
  failures=()
  fetched=0
  for url in "${urls[@]}"; do
    printf 'fetching %s from %s\n' "$name" "$url" >&2
    if ! "$curl_bin" -fsSL "$url" -o "$tmp"; then
      failures+=("$url: download failed")
      continue
    fi
    actual="$(sha256_file "$tmp")"
    if [[ "$actual" != "$expected" ]]; then
      failures+=("$url: SHA-256 mismatch (expected $expected, got $actual)")
      continue
    fi
    mv "$tmp" "$path"
    fetched=1
    break
  done
  if [[ "$fetched" -eq 0 ]]; then
    rm -f "$tmp"
    printf -v joined '%s; ' "${failures[@]}"
    joined="${joined%; }"
    fail "all ${#urls[@]} locators failed for $name: $joined; not writing $path"
  fi
done < "$trip_manifest"
