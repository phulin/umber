#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
texmf_dist="${UMBER_TEXMF_DIST:-${repo_root}/third_party/texlive-20260301-texmf/texmf-dist}"
snapshot_lock="${repo_root}/tests/texlive-snapshot.lock"
pdftex_map="${UMBER_PDFTEX_MAP:-}"
package_database=""
without_package_database=0
output_dir="${repo_root}/target/texlive-snapshot"
objects_base_url="https://example.invalid/umber/texlive/objects/"
shard_bits=8

usage() {
  cat <<'EOF'
usage: scripts/build-texlive-snapshot.sh [--texmf-dist PATH]
       [--pdftex-map PATH]
       [--package-database PATH] [--output-dir PATH]
       [--without-package-database]
       [--objects-base-url HTTPS-URL]
       [--shard-bits BITS]

Builds the full runtime-requestable TeX Live snapshot. Documentation and
source trees are excluded; TeX inputs, TFM metrics, maps, encodings, font
programs, virtual fonts, generated Umber formats, and package hints are kept.
EOF
}

fail() {
  printf 'build-texlive-snapshot.sh: %s\n' "$*" >&2
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
    --texmf-dist) texmf_dist="${2:-}"; shift 2 ;;
    --pdftex-map) pdftex_map="${2:-}"; shift 2 ;;
    --package-database) package_database="${2:-}"; shift 2 ;;
    --without-package-database) without_package_database=1; shift ;;
    --output-dir) output_dir="${2:-}"; shift 2 ;;
    --objects-base-url) objects_base_url="${2:-}"; shift 2 ;;
    --shard-bits) shard_bits="${2:-}"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) fail "unknown option: $1" ;;
  esac
done

[[ -d "$texmf_dist" ]] || fail "missing texmf-dist root: $texmf_dist"
[[ -f "$snapshot_lock" ]] || fail "missing immutable snapshot lock: $snapshot_lock"
if [[ "$without_package_database" -eq 1 ]]; then
  [[ -z "$package_database" ]] || fail "--without-package-database conflicts with --package-database"
else
  package_database="${package_database:-$(dirname "$texmf_dist")/tlpkg/texlive.tlpdb}"
  [[ -f "$package_database" ]] || fail "missing TeX Live package database: $package_database"
fi
if [[ -z "$pdftex_map" ]]; then
  pdftex_map="$(dirname "$texmf_dist")/texmf-var/fonts/map/pdftex/updmap/pdftex.map"
  if [[ ! -f "$pdftex_map" ]]; then
    pdftex_map="$texmf_dist/fonts/map/pdftex/updmap/pdftex.map"
  fi
fi
[[ -f "$pdftex_map" ]] || fail "missing generated pdfTeX map: $pdftex_map"
[[ "$objects_base_url" == https://*/ ]] || fail "objects base URL must be HTTPS and end with /"
[[ "$shard_bits" =~ ^([0-9]|1[0-6])$ ]] || fail "shard bits must be between 0 and 16"

expected_distribution="$(awk '$1 == "distribution" { print $2 }' "$snapshot_lock")"
expected_tree_hash="$(awk '$1 == "tree_sha256" { print $2 }' "$snapshot_lock")"
[[ -n "$expected_distribution" && -n "$expected_tree_hash" ]] || \
  fail "snapshot lock is missing distribution or tree_sha256"
while read -r kind relative expected_bytes expected_hash extra; do
  [[ "$kind" == source ]] || continue
  [[ -z "${extra:-}" && "$relative" != /* && "$relative" != *..* && "$relative" != *\\* ]] || \
    fail "invalid snapshot lock source record: $relative"
  source="$texmf_dist/$relative"
  [[ -f "$source" ]] || fail "missing locked snapshot source: $source"
  actual_bytes="$(wc -c < "$source" | tr -d ' ')"
  actual_hash="$(sha256 "$source")"
  [[ "$actual_bytes" == "$expected_bytes" && "$actual_hash" == "$expected_hash" ]] || \
    fail "snapshot source differs from lock: $source"
done < "$snapshot_lock"

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-texlive-snapshot.XXXXXX")"
cleanup() {
  local status=$?
  if [[ $status -eq 0 ]]; then
    rm -rf "$tmp_root"
  else
    printf 'build-texlive-snapshot.sh: retained failed build at %s\n' "$tmp_root" >&2
  fi
}
trap cleanup EXIT

format_dir="$tmp_root/formats"
"$repo_root/scripts/build-latex-format.sh" \
  --engine latex \
  --publish-input-closure \
  --texmf-dist "$texmf_dist" \
  --output-dir "$format_dir/latex"
"$repo_root/scripts/build-latex-format.sh" \
  --engine pdflatex \
  --publish-input-closure \
  --texmf-dist "$texmf_dist" \
  --output-dir "$format_dir/pdflatex"

# Repository-local configuration inputs are part of the verified format traces,
# but not the pinned TeX Live tree. Stage them as a second deterministic TEXMF
# root so every published closure key resolves to authenticated snapshot bytes.
local_root="$tmp_root/local-format-inputs"
mkdir -p "$local_root/tex"
while read -r kind relative _; do
  case "$kind" in
    local|pdflatex-local)
      cp "$repo_root/$relative" "$local_root/tex/${relative##*/}"
      ;;
  esac
done < "$repo_root/tests/latex-source.lock"

generated_runtime="$tmp_root/generated-runtime"
generated_map_dir="$generated_runtime/fonts/map/pdftex/updmap"
mkdir -p "$generated_map_dir"
install -m 0644 "$pdftex_map" "$generated_map_dir/pdftex.map"

cd "$repo_root"
cargo build -q --release --manifest-path tools/texlive-wasm-publish/Cargo.toml
publisher="${CARGO_TARGET_DIR:-${repo_root}/tools/texlive-wasm-publish/target}/release/texlive-wasm-publish"
tree_hash="$($publisher --tree-sha256 "$texmf_dist")"
[[ "$tree_hash" == "$expected_tree_hash" ]] || \
  fail "texmf-dist tree differs from immutable snapshot lock: expected $expected_tree_hash, got $tree_hash"
local_tree_hash="$($publisher --tree-sha256 "$local_root")"
generated_tree_hash="$($publisher --tree-sha256 "$generated_runtime")"
distribution="$(awk '$1 == "distribution" { print $2 }' tests/latex-source.lock)"
[[ "$distribution" == "$expected_distribution" ]] || \
  fail "format source lock distribution $distribution differs from snapshot lock $expected_distribution"

package_database_entry=""
if [[ -n "$package_database" ]]; then
  package_database_entry="$(printf ',\n  "packageDatabase": "%s"' "$package_database")"
fi

config="$tmp_root/publish.json"
cat > "$config" <<EOF
{
  "schema": 3,
  "distribution": "${distribution}",
  "objectsBaseUrl": "${objects_base_url}",
  "shardBits": ${shard_bits},
  "roots": [
    {
      "name": "texlive-runtime",
      "path": "${texmf_dist}",
      "treeSha256": "${tree_hash}"
    },
    {
      "name": "format-local-inputs",
      "path": "${local_root}",
      "treeSha256": "${local_tree_hash}"
    },
    {
      "name": "texlive-generated-runtime",
      "path": "${generated_runtime}",
      "treeSha256": "${generated_tree_hash}"
    }
  ]${package_database_entry},
  "inventory": {
    "minimumLogicalFiles": 100000,
    "minimumObjects": 50000,
    "minimumBytes": 1000000000
  },
  "formats": [
    {
      "path": "${format_dir}/latex/latex.fmt",
      "metadata": "${format_dir}/latex/latex-format.json"
    },
    {
      "path": "${format_dir}/pdflatex/pdflatex.fmt",
      "metadata": "${format_dir}/pdflatex/pdflatex-format.json"
    }
  ]
}
EOF

first="$tmp_root/first"
"$publisher" "$config" "$first"
"$publisher" "$config" "$output_dir"
diff -qr "$first" "$output_dir" >/dev/null || fail "two clean publications differ"

shards="$(jq '.shardCount' "$output_dir/manifest.json")"
objects="$(find "$output_dir/objects" -type f | wc -l | tr -d ' ')"
bytes="$(find "$output_dir/objects" -type f -exec stat -f '%z' {} + | awk '{ total += $1 } END { print total + 0 }')"
manifest_digest="$(sha256 "$output_dir/manifest.json")"
printf 'TeX Live snapshot: shards=%s objects=%s bytes=%s root_sha256=%s output=%s\n' \
  "$shards" "$objects" "$bytes" "$manifest_digest" "$output_dir"
