#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 3 ]]; then
  printf 'usage: %s ETEX.CH AUDIT EXTENSION-EVENT-MATRIX\n' "$0" >&2
  exit 2
fi

etex_change="$1"
audit="$2"
matrix="$3"

fail() {
  printf 'audit-etex26-extension-primitives: %s\n' "$*" >&2
  exit 1
}

for input in "$etex_change" "$audit" "$matrix"; do
  [[ -f "$input" ]] || fail "missing input: $input"
done

work_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-etex26-primitive-audit.XXXXXX")"
trap 'rm -rf "$work_root"' EXIT
source_inventory="${work_root}/source"
audit_inventory="${work_root}/audit"

awk 'match($0,/primitive\("[^"]+"/) {
  print substr($0,RSTART+11,RLENGTH-12)
}' "$etex_change" | LC_ALL=C sort -u >"$source_inventory"
[[ -s "$source_inventory" ]] ||
  fail "canonical source declares no primitives: $etex_change"

while IFS='|' read -r primitive owner gate seam extra; do
  [[ -z "$primitive" || "$primitive" == \#* ]] && continue
  [[ -n "$owner" && -n "$gate" && -n "$seam" && -z "${extra:-}" ]] ||
    fail "malformed audit row for ${primitive:-unknown}"
  [[ "$owner" == command-core || "$owner" == executor ]] ||
    fail "unknown owner for $primitive: $owner"
  if [[ "$owner" == command-core ]]; then
    awk -F'|' -v gate="$gate" '$2 == gate { found=1 } END { exit !found }' \
      "$matrix" ||
      fail "command-core primitive $primitive has no extension matrix boundary: $gate"
  fi
  printf '%s\n' "$primitive"
done <"$audit" | LC_ALL=C sort >"$audit_inventory"

duplicate="$(uniq -d "$audit_inventory" | head -1)"
[[ -z "$duplicate" ]] || fail "duplicate audit primitive: $duplicate"

missing="$(comm -23 "$source_inventory" "$audit_inventory" | paste -sd, -)"
extra="$(comm -13 "$source_inventory" "$audit_inventory" | paste -sd, -)"
[[ -z "$missing" ]] || fail "audit is missing canonical primitives: $missing"
[[ -z "$extra" ]] || fail "audit contains noncanonical primitives: $extra"

printf 'e-TeX extension primitive audit passed (%s canonical primitives)\n' \
  "$(wc -l <"$source_inventory" | tr -d ' ')"
