#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if ! command -v hb-shape >/dev/null 2>&1; then
  echo "hb-shape not found; skipping optional rustybuzz fixture diagnostic"
  exit 0
fi

actual=$(mktemp "${TMPDIR:-/tmp}/umber-hb-shape.XXXXXX")
trap 'rm -f "$actual"' EXIT HUP INT TERM

{
  echo "arabic-mark"
  hb-shape "$repo_root/crates/tex-shape/tests/fixtures/NotoSansArabic.ttf" 'لَا' \
    --direction=rtl --script=arab --language=ar --features=kern=1,liga=1 \
    --no-glyph-names
  echo "devanagari-conjunct"
  hb-shape "$repo_root/crates/tex-shape/tests/fixtures/NotoSansDevanagari.ttf" 'क्षि' \
    --direction=ltr --script=deva --language=hi --features=kern=1,liga=1 \
    --no-glyph-names
} > "$actual"

if ! diff -u "$repo_root/crates/tex-shape/tests/fixtures/hb-shape.expected" "$actual"; then
  echo "hb-shape differs from the committed rustybuzz cross-check fixture" >&2
  exit 1
fi
echo "hb-shape fixtures agree"
