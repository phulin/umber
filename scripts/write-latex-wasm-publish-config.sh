#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 8 ]]; then
  printf '%s\n' \
    'usage: write-latex-wasm-publish-config.sh OUTPUT DISTRIBUTION OBJECTS-BASE-URL RUNTIME-ROOT TREE-AHASH64 FORMAT METADATA INPUT-IDENTITIES' >&2
  exit 2
fi

output="$1"
distribution="$2"
objects_base_url="$3"
runtime_root="$4"
tree_ahash64="$5"
format_path="$6"
metadata_path="$7"
input_identities_path="$8"

# Keep the focused LaTeX bundle on the measured production distribution layout.
# docs/distribution_manifest.md records the 256-shard selection evidence.
shard_bits=8

cat > "$output" <<EOF
{
  "schema": 8,
  "distribution": "${distribution}",
  "objectsBaseUrl": "${objects_base_url}",
  "shardBits": ${shard_bits},
  "roots": [
    {
      "name": "latex-base-runtime",
      "path": "${runtime_root}",
      "treeAhash64": "${tree_ahash64}"
    }
  ],
  "dependencies": {
    "tex:article.cls": ["tex:size10.clo", "tex:l3backend-dvips.def"],
    "tex:book.cls": ["tex:bk10.clo", "tex:l3backend-dvips.def"],
    "tex:letter.cls": ["tex:size10.clo", "tex:l3backend-dvips.def"],
    "tex:report.cls": ["tex:size10.clo", "tex:l3backend-dvips.def"]
  },
  "formats": [
    {
      "path": "${format_path}",
      "metadata": "${metadata_path}",
      "inputIdentities": "${input_identities_path}"
    }
  ]
}
EOF
