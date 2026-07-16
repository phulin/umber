#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
staging="/private/tmp/umber-texlive-2026/staging/texlive-2026-r79639"
snapshot="texlive/texlive-2026-r79639"
bucket="umber-assets"
public_origin="https://assets.umber.ink"
env_file="$repo_root/.env"
rclone="${RCLONE:-rclone}"
curl="${CURL:-curl}"
transfers=8
checkers=16
retries=5
expected_objects=153897
expected_bytes=3507703184
expected_manifest_sha256="602736c8d6f745972ad5d61acfab90b20ed0f4e67fd3b02a8ff7d260a34dee60"
dry_run=0

usage() {
  cat <<'EOF'
usage: scripts/publish-texlive-r2.sh [options]

Resume-safe, manifest-last publication of the verified TeX Live 2026 staging
bundle to Cloudflare R2. The destination defaults to
umber-assets:texlive/texlive-2026-r79639.

options:
  --staging PATH                 staged bundle containing objects/ and manifest.json
  --snapshot PREFIX             immutable bucket prefix
  --bucket NAME                 R2 bucket (default: umber-assets)
  --public-origin HTTPS-ORIGIN  public custom-domain origin
  --env-file PATH               ignored dotenv file containing credentials
  --transfers N                 concurrent object transfers (default: 8)
  --checkers N                  concurrent remote checks (default: 16)
  --retries N                   high-level retry attempts (default: 5)
  --dry-run                     validate and show rclone's transfer plan only
  --expected-objects N          exact staged/remote object count
  --expected-bytes N            exact staged/remote object bytes
  --expected-manifest-sha256 H  exact manifest digest
  --rclone PATH                 rclone executable
  --curl PATH                   curl executable

The dotenv file must define CLOUDFLARE_ACCOUNT_ID, R2_ACCESS_KEY_ID, and
R2_SECRET_ACCESS_KEY. It is parsed as data, never sourced. Credentials are
passed to rclone only through its per-process environment configuration.
EOF
}

fail() {
  printf 'publish-texlive-r2.sh: %s\n' "$*" >&2
  exit 1
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

file_size() {
  if stat -f '%z' "$1" >/dev/null 2>&1; then
    stat -f '%z' "$1"
  else
    stat -c '%s' "$1"
  fi
}

positive_integer() {
  [[ "$1" =~ ^[1-9][0-9]*$ ]]
}

dotenv_value() {
  local key="$1" line value=""
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%$'\r'}"
    if [[ "$line" == "$key="* ]]; then
      value="${line#*=}"
    elif [[ "$line" == "export $key="* ]]; then
      value="${line#*=}"
    else
      continue
    fi
    if [[ ${#value} -ge 2 ]]; then
      if [[ "${value:0:1}" == '"' && "${value: -1}" == '"' ]] ||
         [[ "${value:0:1}" == "'" && "${value: -1}" == "'" ]]; then
        value="${value:1:${#value}-2}"
      fi
    fi
  done < "$env_file"
  printf '%s' "$value"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --staging) staging="${2:-}"; shift 2 ;;
    --snapshot) snapshot="${2:-}"; shift 2 ;;
    --bucket) bucket="${2:-}"; shift 2 ;;
    --public-origin) public_origin="${2:-}"; shift 2 ;;
    --env-file) env_file="${2:-}"; shift 2 ;;
    --transfers) transfers="${2:-}"; shift 2 ;;
    --checkers) checkers="${2:-}"; shift 2 ;;
    --retries) retries="${2:-}"; shift 2 ;;
    --expected-objects) expected_objects="${2:-}"; shift 2 ;;
    --expected-bytes) expected_bytes="${2:-}"; shift 2 ;;
    --expected-manifest-sha256) expected_manifest_sha256="${2:-}"; shift 2 ;;
    --dry-run) dry_run=1; shift ;;
    --rclone) rclone="${2:-}"; shift 2 ;;
    --curl) curl="${2:-}"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) fail "unknown option: $1" ;;
  esac
done

[[ -d "$staging/objects" ]] || fail "missing staged objects directory: $staging/objects"
[[ -f "$staging/manifest.json" ]] || fail "missing staged manifest: $staging/manifest.json"
[[ "$snapshot" =~ ^[a-z0-9][a-z0-9._/-]*[a-z0-9]$ ]] || fail "invalid immutable snapshot prefix"
[[ "$snapshot" != *'..'* && "$snapshot" != /* && "$snapshot" != */ ]] || fail "invalid immutable snapshot prefix"
[[ "$bucket" =~ ^[a-z0-9][a-z0-9.-]*[a-z0-9]$ ]] || fail "invalid bucket name"
[[ "$public_origin" == https://* && "$public_origin" != */ ]] || fail "--public-origin must be an HTTPS origin without a trailing slash"
positive_integer "$transfers" || fail "--transfers must be a positive integer"
positive_integer "$checkers" || fail "--checkers must be a positive integer"
positive_integer "$retries" || fail "--retries must be a positive integer"
positive_integer "$expected_objects" || fail "--expected-objects must be a positive integer"
positive_integer "$expected_bytes" || fail "--expected-bytes must be a positive integer"
[[ "$expected_manifest_sha256" =~ ^[0-9a-f]{64}$ ]] || fail "invalid expected manifest SHA-256"
[[ -f "$env_file" ]] || fail "credential file not found: $env_file"
command -v "$rclone" >/dev/null 2>&1 || fail "rclone executable not found: $rclone"
command -v "$curl" >/dev/null 2>&1 || fail "curl executable not found: $curl"

account_id="$(dotenv_value CLOUDFLARE_ACCOUNT_ID)"
access_key_id="$(dotenv_value R2_ACCESS_KEY_ID)"
secret_access_key="$(dotenv_value R2_SECRET_ACCESS_KEY)"
[[ -n "$account_id" ]] || fail "CLOUDFLARE_ACCOUNT_ID is missing from $env_file"
[[ -n "$access_key_id" ]] || fail "R2_ACCESS_KEY_ID is missing from $env_file"
[[ -n "$secret_access_key" ]] || fail "R2_SECRET_ACCESS_KEY is missing from $env_file"

# Keep secrets out of argv, logs, and persistent rclone configuration.
export RCLONE_CONFIG_UMBER_R2_TYPE=s3
export RCLONE_CONFIG_UMBER_R2_PROVIDER=Cloudflare
export RCLONE_CONFIG_UMBER_R2_ACCESS_KEY_ID="$access_key_id"
export RCLONE_CONFIG_UMBER_R2_SECRET_ACCESS_KEY="$secret_access_key"
export RCLONE_CONFIG_UMBER_R2_ENDPOINT="https://${account_id}.r2.cloudflarestorage.com"
export RCLONE_CONFIG_UMBER_R2_NO_CHECK_BUCKET=true
unset access_key_id secret_access_key

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-r2-publish.XXXXXX")"
trap 'rm -rf "$tmp_root"' EXIT

local_inventory="$tmp_root/local-inventory"
while IFS= read -r -d '' object; do
  printf '%s\t%s\n' "$(file_size "$object")" "${object##*/}"
done < <(find "$staging/objects" -type f -print0) > "$local_inventory"
local_objects="$(wc -l < "$local_inventory" | tr -d ' ')"
local_bytes="$(awk -F $'\t' '{ total += $1 } END { printf "%.0f", total }' "$local_inventory")"
[[ "$local_objects" == "$expected_objects" ]] || fail "staged object count $local_objects does not match expected $expected_objects"
[[ "$local_bytes" == "$expected_bytes" ]] || fail "staged object bytes $local_bytes does not match expected $expected_bytes"
actual_manifest_sha256="$(sha256 "$staging/manifest.json")"
[[ "$actual_manifest_sha256" == "$expected_manifest_sha256" ]] || fail "staged manifest digest $actual_manifest_sha256 does not match expected $expected_manifest_sha256"

common_flags=(
  --config /dev/null
  --transfers "$transfers"
  --checkers "$checkers"
  --retries "$retries"
  --low-level-retries 10
  --retries-sleep 10s
  --stats 30s
)
remote_objects="umber_r2:${bucket}/${snapshot}/objects"
remote_manifest="umber_r2:${bucket}/${snapshot}/manifest.json"

printf 'Validated staging: objects=%s bytes=%s manifest_sha256=%s\n' \
  "$local_objects" "$local_bytes" "$actual_manifest_sha256"

copy_flags=(
  --immutable
  --size-only
  --metadata-set 'Cache-Control=public, max-age=31536000, immutable'
  --metadata-set 'Content-Type=application/octet-stream'
)
if (( dry_run == 1 )); then
  "$rclone" copy "$staging/objects" "$remote_objects" \
    "${common_flags[@]}" "${copy_flags[@]}" --dry-run
  printf 'Dry run complete; remote verification and manifest publication were skipped.\n'
  exit 0
fi

# copy never deletes destination objects. --immutable prevents a conflicting
# object at a digest key from being overwritten; rerunning fills only misses.
"$rclone" copy "$staging/objects" "$remote_objects" \
  "${common_flags[@]}" "${copy_flags[@]}"

# The local digest filenames were produced by the verified snapshot builder.
# rclone check compares size and the strongest common remote hash for every
# source object. Inventory equality then rejects missing or extra keys.
"$rclone" check "$staging/objects" "$remote_objects" \
  "${common_flags[@]}" --one-way
remote_inventory="$tmp_root/remote-inventory"
"$rclone" lsf "$remote_objects" "${common_flags[@]}" --recursive --files-only \
  --format sp --separator $'\t' > "$remote_inventory"
remote_count="$(wc -l < "$remote_inventory" | tr -d ' ')"
remote_bytes="$(awk -F $'\t' '{ total += $1 } END { printf "%.0f", total }' "$remote_inventory")"
[[ "$remote_count" == "$expected_objects" ]] || fail "remote object count $remote_count does not match expected $expected_objects"
[[ "$remote_bytes" == "$expected_bytes" ]] || fail "remote object bytes $remote_bytes does not match expected $expected_bytes"

# This is intentionally the first manifest write in the script.
"$rclone" copyto "$staging/manifest.json" "$remote_manifest" \
  "${common_flags[@]}" --immutable --checksum \
  --metadata-set 'Cache-Control=public, max-age=31536000, immutable' \
  --metadata-set 'Content-Type=application/json'

public_prefix="${public_origin}/${snapshot}"
headers="$tmp_root/headers"
public_manifest="$tmp_root/manifest.json"
"$curl" --fail --silent --show-error \
  --retry 10 --retry-all-errors --retry-delay 5 \
  --header 'Origin: https://browser.example' \
  --dump-header "$headers" \
  "$public_prefix/manifest.json" \
  --output "$public_manifest"
public_manifest_sha256="$(sha256 "$public_manifest")"
[[ "$public_manifest_sha256" == "$expected_manifest_sha256" ]] || fail "public manifest digest $public_manifest_sha256 does not match expected $expected_manifest_sha256"
grep -Eiq '^access-control-allow-origin:[[:space:]]*\*' "$headers" || fail "public manifest response does not allow cross-origin browser access"

# Verify deterministic representatives spread across the sorted object set.
object_names="$tmp_root/object-names"
find "$staging/objects" -type f -exec basename {} \; | LC_ALL=C sort > "$object_names"
representative_lines=(1 "$(( (expected_objects + 1) / 2 ))" "$expected_objects")
for line in "${representative_lines[@]}"; do
  name="$(sed -n "${line}p" "$object_names")"
  public_object="$tmp_root/$name"
  "$curl" --fail --silent --show-error \
    --retry 10 --retry-all-errors --retry-delay 5 \
    --header 'Origin: https://browser.example' \
    --dump-header "$headers" \
    "$public_prefix/objects/$name" \
    --output "$public_object"
  [[ "$(sha256 "$public_object")" == "${name#sha256-}" ]] || fail "public object $name failed digest verification"
  grep -Eiq '^access-control-allow-origin:[[:space:]]*\*' "$headers" || fail "public object $name does not allow cross-origin browser access"
done

printf 'Published %s: objects=%s bytes=%s manifest=%s digest=%s\n' \
  "$snapshot" "$remote_count" "$remote_bytes" "$public_prefix/manifest.json" "$expected_manifest_sha256"
