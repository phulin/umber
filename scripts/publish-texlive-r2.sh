#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
texmf_dist="${UMBER_TEXMF_DIST:-/usr/local/texlive/2025/texmf-dist}"
bucket=""
snapshot=""
public_prefix=""
create_bucket=0
wrangler="${WRANGLER:-wrangler}"

usage() {
  cat <<'EOF'
usage: scripts/publish-texlive-r2.sh --bucket NAME --snapshot ID --public-prefix HTTPS-URL [options]

Build and publish an immutable TeX Live snapshot to Cloudflare R2. PUBLIC-PREFIX
is the public bucket/custom-domain root; assets are written beneath SNAPSHOT/.

options:
  --texmf-dist PATH   pinned TeX Live texmf-dist tree
  --create-bucket     create NAME before configuring and uploading it
  --wrangler PATH     Wrangler executable (default: $WRANGLER or wrangler)
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

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bucket) bucket="${2:-}"; shift 2 ;;
    --snapshot) snapshot="${2:-}"; shift 2 ;;
    --public-prefix) public_prefix="${2:-}"; shift 2 ;;
    --texmf-dist) texmf_dist="${2:-}"; shift 2 ;;
    --create-bucket) create_bucket=1; shift ;;
    --wrangler) wrangler="${2:-}"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) fail "unknown option: $1" ;;
  esac
done

[[ -n "$bucket" ]] || fail "--bucket is required"
[[ "$snapshot" =~ ^[a-z0-9][a-z0-9._-]*$ ]] || fail "--snapshot must be a stable lowercase identifier"
[[ "$public_prefix" == https://*/ ]] || fail "--public-prefix must be HTTPS and end with /"
[[ -x "$(command -v "$wrangler" 2>/dev/null || true)" ]] || fail "Wrangler executable not found: $wrangler"

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-r2-publish.XXXXXX")"
cleanup() {
  local status=$?
  if [[ $status -eq 0 ]]; then
    rm -rf "$tmp_root"
  else
    printf 'publish-texlive-r2.sh: retained failed publication at %s\n' "$tmp_root" >&2
  fi
}
trap cleanup EXIT

snapshot_prefix="${public_prefix}${snapshot}/"
bundle="$tmp_root/bundle"
"$repo_root/scripts/build-wasm-latex-bundle.sh" \
  --texmf-dist "$texmf_dist" \
  --output-dir "$bundle" \
  --objects-base-url "${snapshot_prefix}objects/"

if [[ $create_bucket == 1 ]]; then
  "$wrangler" r2 bucket create "$bucket"
fi
"$wrangler" r2 bucket cors set "$bucket" --file "$repo_root/scripts/texlive-r2-cors.json"

while IFS= read -r object; do
  name="$(basename "$object")"
  "$wrangler" r2 object put "$bucket/$snapshot/objects/$name" \
    --file "$object" \
    --content-type application/octet-stream \
    --cache-control 'public, max-age=31536000, immutable'
done < <(find "$bundle/objects" -type f -print | LC_ALL=C sort)

# Publish the manifest last so a visible snapshot is always complete.
"$wrangler" r2 object put "$bucket/$snapshot/manifest.json" \
  --file "$bundle/manifest.json" \
  --content-type application/json \
  --cache-control 'public, max-age=31536000, immutable'

remote_manifest="$tmp_root/manifest.json"
headers="$tmp_root/headers"
curl --fail --silent --show-error \
  --header 'Origin: https://browser.example' \
  --dump-header "$headers" \
  "${snapshot_prefix}manifest.json" \
  --output "$remote_manifest"
expected="$(sha256 "$bundle/manifest.json")"
actual="$(sha256 "$remote_manifest")"
[[ "$actual" == "$expected" ]] || fail "published manifest digest $actual does not match $expected"
grep -Eiq '^access-control-allow-origin:[[:space:]]*\*' "$headers" || \
  fail "public manifest response does not allow cross-origin browser access"

remote_object="$tmp_root/object"
while IFS= read -r object; do
  name="$(basename "$object")"
  curl --fail --silent --show-error \
    "${snapshot_prefix}objects/$name" \
    --output "$remote_object"
  [[ "$(sha256 "$remote_object")" == "${name#sha256-}" ]] || \
    fail "published object $name failed digest verification"
done < <(find "$bundle/objects" -type f -print | LC_ALL=C sort)

printf 'Published %s: manifest=%s digest=%s\n' "$snapshot" "${snapshot_prefix}manifest.json" "$expected"
