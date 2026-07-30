#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT

mkdir -p "$temporary/candidate"
printf 'complete\n' >"$temporary/candidate/marker"
"$repo_root/scripts/build-tex82-document-traces.sh" \
  --test-publish-candidate "$temporary/candidate" "$temporary/staged/plain"
[[ "$(cat "$temporary/staged/plain/marker")" == complete ]]

mkdir -p "$temporary/bin" "$temporary/failing-candidate"
printf '#!/usr/bin/env bash\nexit 17\n' >"$temporary/bin/mv"
chmod +x "$temporary/bin/mv"
if PATH="$temporary/bin:$PATH" "$repo_root/scripts/build-tex82-document-traces.sh" \
  --test-publish-candidate "$temporary/failing-candidate" "$temporary/failing/plain" \
  >"$temporary/stdout" 2>"$temporary/stderr"; then
  printf 'test-build-tex82-document-traces: staging failure returned success\n' >&2
  exit 1
fi
grep -F 'could not stage the document trace candidate' "$temporary/stderr" >/dev/null

printf 'test-build-tex82-document-traces: PASS\n'
