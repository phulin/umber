#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
texmf_dist="${UMBER_TEXMF_DIST:-${repo_root}/third_party/texlive-2026/texmf-dist}"
snapshot_lock="${repo_root}/tests/texlive-snapshot.lock"
output_dir="${repo_root}/target/html-r2"
objects_base_url="https://assets.umber.ink/html/umber-html-mvp-v1/objects/"
shard_bits=4

usage() {
  cat <<'EOF'
usage: scripts/build-html-r2.sh [--texmf-dist PATH] [--output-dir PATH]
       [--objects-base-url HTTPS-URL] [--shard-bits BITS]

Builds two byte-identical copies of the immutable contract-v1 HTML-only R2
profile. The result contains the authenticated LaTeX/pdfLaTeX runtime closure,
the curated cmr10/CMU/STIX catalog, and no DVI/PDF-only font material.
EOF
}

fail() {
  printf 'build-html-r2.sh: %s\n' "$*" >&2
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
    --output-dir) output_dir="${2:-}"; shift 2 ;;
    --objects-base-url) objects_base_url="${2:-}"; shift 2 ;;
    --shard-bits) shard_bits="${2:-}"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) fail "unknown option: $1" ;;
  esac
done

[[ -d "$texmf_dist" ]] || fail "missing texmf-dist root: $texmf_dist"
[[ -f "$snapshot_lock" ]] || fail "missing immutable snapshot lock: $snapshot_lock"
[[ "$objects_base_url" == https://*/ ]] || fail "objects base URL must be HTTPS and end with /"
[[ "$shard_bits" =~ ^([0-9]|1[0-6])$ ]] || fail "shard bits must be between 0 and 16"

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-html-r2.XXXXXX")"
cleanup() {
  local status=$?
  if [[ $status -eq 0 ]]; then
    rm -rf "$tmp_root"
  else
    printf 'build-html-r2.sh: retained failed build at %s\n' "$tmp_root" >&2
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

# The format trace includes two repository-owned configuration inputs. Give
# them an immutable publisher root so every closure key resolves exactly.
local_root="$tmp_root/local-format-inputs"
mkdir -p "$local_root/tex"
while read -r kind relative _; do
  case "$kind" in
    local|pdflatex-local)
      cp "$repo_root/$relative" "$local_root/tex/${relative##*/}"
      ;;
  esac
done < "$repo_root/tests/latex-source.lock"

cd "$repo_root"
cargo build -q --release --manifest-path tools/texlive-wasm-publish/Cargo.toml
publisher="${CARGO_TARGET_DIR:-${repo_root}/tools/texlive-wasm-publish/target}/release/texlive-wasm-publish"
expected_tree_hash="$(awk '$1 == "tree_sha256" { print $2 }' "$snapshot_lock")"
tree_hash="$($publisher --tree-sha256 "$texmf_dist")"
[[ "$tree_hash" == "$expected_tree_hash" ]] || \
  fail "texmf-dist tree differs from immutable snapshot lock: expected $expected_tree_hash, got $tree_hash"
local_tree_hash="$($publisher --tree-sha256 "$local_root")"

config="$tmp_root/publish.json"
jq -n \
  --arg objects "$objects_base_url" \
  --arg texmf "$texmf_dist" \
  --arg texmf_hash "$tree_hash" \
  --arg local "$local_root" \
  --arg local_hash "$local_tree_hash" \
  --arg latex "$format_dir/latex/latex.fmt" \
  --arg latex_meta "$format_dir/latex/latex-format.json" \
  --arg pdflatex "$format_dir/pdflatex/pdflatex.fmt" \
  --arg pdflatex_meta "$format_dir/pdflatex/pdflatex-format.json" \
  --arg catalog "$repo_root/tools/texlive-wasm-publish/catalog/html-mvp-v1.json" \
  --arg cmu "$repo_root/crates/umber-wasm/assets/cmu-serif-500-roman.woff2" \
  --arg cmu_license "$repo_root/crates/umber-wasm/assets/CMU-OFL.txt" \
  --arg stix "$repo_root/crates/tex-fonts/tests/fixtures/stix-two-math.woff2" \
  --arg stix_license "$repo_root/crates/tex-fonts/tests/fixtures/stix-two-math.LICENSE.txt" \
  --argjson shard_bits "$shard_bits" \
  '{
    schema: 4,
    distribution: "umber-html-mvp-v1",
    objectsBaseUrl: $objects,
    shardBits: $shard_bits,
    profile: "html",
    roots: [
      {name: "texlive-runtime", path: $texmf, treeSha256: $texmf_hash},
      {name: "format-local-inputs", path: $local, treeSha256: $local_hash}
    ],
    formats: [
      {path: $latex, metadata: $latex_meta},
      {path: $pdflatex, metadata: $pdflatex_meta}
    ],
    html: {
      runtimeFileKeys: [],
      catalog: $catalog,
      objectSources: {
        "1b875e541dc5c517cd11d244710d8639addbe91a0bb1ba55e7c4593225c7a970": $cmu,
        "73273dffdefe2e5f1e138084d4a4b65b1c50df2ab0179f78484f31beefe30d84": $cmu_license,
        "cb1149b7c8b7b194eff7f42e20cf9e7a9706d342ffc2b14765624577d8be38e3": $stix,
        "0c8825913b60d858aacdb33c4ca6660a7d64b0d6464702efbb19313f5765861a": $stix_license
      },
      inventory: {
        maximumLogicalFiles: 128,
        maximumObjects: 160,
        maximumBytes: 33554432,
        maximumFonts: 2,
        maximumLegacyMappings: 1,
        maximumLicenses: 2
      }
    }
  }' > "$config"

first="$tmp_root/first"
"$publisher" "$config" "$first"
"$publisher" "$config" "$output_dir"
diff -qr "$first" "$output_dir" >/dev/null || fail "two clean HTML publications differ"
"$publisher" --verify-sharded "$output_dir"

objects="$(find "$output_dir/objects" -type f | wc -l | tr -d ' ')"
bytes="$(find "$output_dir/objects" -type f -exec stat -f '%z' {} + | awk '{ total += $1 } END { print total + 0 }')"
manifest_digest="$(sha256 "$output_dir/manifest.json")"
printf 'HTML R2 staging: shards=%s objects=%s bytes=%s root_sha256=%s output=%s\n' \
  "$((1 << shard_bits))" "$objects" "$bytes" "$manifest_digest" "$output_dir"
