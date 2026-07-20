#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 7 ]]; then
  printf '%s\n' \
    'usage: write-latex-wasm-publish-config.sh OUTPUT DISTRIBUTION OBJECTS-BASE-URL RUNTIME-ROOT TREE-SHA256 FORMAT METADATA' >&2
  exit 2
fi

output="$1"
distribution="$2"
objects_base_url="$3"
runtime_root="$4"
tree_sha256="$5"
format_path="$6"
metadata_path="$7"

# Keep the focused LaTeX bundle on the measured production distribution layout.
# docs/distribution_manifest.md records the 256-shard selection evidence.
shard_bits=8

cat > "$output" <<EOF
{
  "schema": 3,
  "distribution": "${distribution}",
  "objectsBaseUrl": "${objects_base_url}",
  "shardBits": ${shard_bits},
  "roots": [
    {
      "name": "latex-base-runtime",
      "path": "${runtime_root}",
      "treeSha256": "${tree_sha256}"
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
      "metadata": "${metadata_path}"
    }
  ]
}
EOF
