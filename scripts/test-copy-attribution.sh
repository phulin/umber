#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_root="$repo_root/scripts/copy-attribution"
work_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-copy-attribution.XXXXXX")"
trap 'rm -rf "$work_root"' EXIT

cc -shared -fPIC -O2 -g -fno-builtin-memcpy -fno-builtin-memmove \
  -Wall -Wextra -Werror -o "$work_root/copy_attribution_probe.so" \
  "$fixture_root/copy_attribution_probe.c" -ldl
cc -shared -fPIC -O2 -g -fno-builtin-memmove -fno-optimize-sibling-calls \
  -Wall -Wextra -Werror -o "$work_root/libexternal_memmove.so" \
  "$fixture_root/external_memmove.c" -pthread
rustc --edition=2024 -C opt-level=2 -C debuginfo=2 -C force-frame-pointers=yes \
  -C link-arg=-Wl,--export-dynamic -C link-arg=-Wl,-rpath,"$work_root" \
  -C link-arg=-L"$work_root" -C link-arg=-lexternal_memmove -C link-arg=-pthread \
  -o "$work_root/copy-attribution-microgate" "$fixture_root/microgate.rs"

UMBER_COPY_ATTRIBUTION_OUT="$work_root/raw.txt" \
  LD_PRELOAD="$work_root/copy_attribution_probe.so" \
  "$work_root/copy-attribution-microgate" >"$work_root/stdout.txt"
"$fixture_root/symbolize.py" \
  --binary "$work_root/copy-attribution-microgate" \
  --report "$work_root/raw.txt" --limit 100 >"$work_root/symbolized.txt"

grep -q 'TOTAL api=memcpy ' "$work_root/symbolized.txt"
grep -q 'TOTAL api=memmove ' "$work_root/symbolized.txt"
grep -q 'function=.*scalar_copy_gate' "$work_root/symbolized.txt"
grep -q 'function=.*vec_copy_gate' "$work_root/symbolized.txt"
grep -q 'class=application_ancestor' "$work_root/symbolized.txt"
grep -q 'function=.*external_memmove_ancestor_gate' "$work_root/symbolized.txt"
grep -q 'class=external_only' "$work_root/symbolized.txt"
grep -q 'EXTERNAL module=.*libexternal_memmove' "$work_root/symbolized.txt"
grep -Eq 'COPY_TABLE api=memcpy .*overflow_calls=0 ' "$work_root/symbolized.txt"
grep -Eq 'COPY_TABLE api=memmove .*overflow_calls=0 ' "$work_root/symbolized.txt"

printf 'copy attribution microgate: PASS (scalar, Vec, external ancestor/only, exact totals)\n'
