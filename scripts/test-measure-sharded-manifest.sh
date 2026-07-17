#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/umber-shard-measure-test.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/traces/a"

cat >"$fixture/manifest.json" <<'EOF'
{"schema":1,"distribution":"test","objectsBaseUrl":"https://example.test/objects/","files":{"tex:a.sty":{"virtualPath":"/texlive/tex/latex/a/a.sty","object":"sha256-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","bytes":1,"dependencies":["tex:b.sty"]},"tex:b.sty":{"virtualPath":"/texlive/tex/latex/b/b.sty","object":"sha256-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","bytes":2,"dependencies":[]}},"fonts":{},"formats":{}}
EOF
printf '/texlive/tex/latex/a/a.sty\n' >"$fixture/traces/a/files.txt"

python3 "$repo_root/scripts/measure-sharded-manifest.py" \
  "$fixture/manifest.json" "$fixture/traces" --shard-bits 0,1 \
  --output "$fixture/first.tsv"
python3 "$repo_root/scripts/measure-sharded-manifest.py" \
  "$fixture/manifest.json" "$fixture/traces" --shard-bits 0,1 \
  --output "$fixture/second.tsv"
cmp "$fixture/first.tsv" "$fixture/second.tsv"
awk -F '\t' '
  /^# trace_files/ { if ($2 != 1) exit 1; traces = 1 }
  /^# matched_lookup_keys/ { if ($2 != 1) exit 1; keys = 1 }
  $1 == 0 { if ($2 != 1 || $4 != 2 || $5 != 1 || $7 != 2 || $8 != 1) exit 1; zero = 1 }
  $1 == 1 { if ($2 != 2 || $4 != 2 || $5 != 1 || $7 != 2 || $8 != 1) exit 1; one = 1 }
  END { if (!traces || !keys || !zero || !one) exit 1 }
' "$fixture/first.tsv"
echo "measure-sharded-manifest test passed"
