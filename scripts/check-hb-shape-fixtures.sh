#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Cross-check the committed rustybuzz mark and conjunct fixtures against C
# HarfBuzz.
#
# This used to print "hb-shape not found; skipping" and exit 0, so the status of
# a run in which the comparison never happened was byte-identical to the status
# of a run in which every fixture matched (umber2-johp.210). That is the defect
# this repository keeps finding: a check believed to be running that is not. A
# missing `hb-shape` is now BLOCKED, which is never 0, and the run is stamped
# where `scripts/check.sh` and `scripts/run-native-tests.py` print it.

# shellcheck source=scripts/tier-runner.sh
source "$repo_root/scripts/tier-runner.sh"

TIER_ARGS="$*" tier_begin check-hb-shape-fixtures.sh hb-shape-comparison

fixtures="$repo_root/crates/tex-shape/tests/fixtures"

compare_fixtures() {
  local actual status
  actual=$(mktemp "${TMPDIR:-/tmp}/umber-hb-shape.XXXXXX") || return 1
  {
    echo "arabic-mark"
    hb-shape "$fixtures/NotoSansArabic.ttf" 'لَا' \
      --direction=rtl --script=arab --language=ar --features=kern=1,liga=1 \
      --no-glyph-names
    echo "devanagari-conjunct"
    hb-shape "$fixtures/NotoSansDevanagari.ttf" 'क्षि' \
      --direction=ltr --script=deva --language=hi --features=kern=1,liga=1 \
      --no-glyph-names
  } >"$actual"

  status=0
  if diff -u "$fixtures/hb-shape.expected" "$actual"; then
    echo "hb-shape agrees with both committed rustybuzz fixtures"
  else
    echo "hb-shape differs from the committed rustybuzz cross-check fixture" >&2
    status=1
  fi
  rm -f "$actual"
  return "$status"
}

tier_step_requiring "hb-shape mktemp diff" hb-shape-comparison compare_fixtures

tier_finish
