#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

scripts/test-etex26-extension-primitive-audit.sh

scripts/regen-fixtures.sh --oracle tex82 --profile initex-eight-bit --validate-only
scripts/regen-fixtures.sh --oracle tex82 --profile initex-eight-bit \
  --fixture tex82/command-transitions-v1 --validate-only
scripts/regen-fixtures.sh --oracle etex26 \
  --profile compatibility+extended-eight-bit --validate-only
scripts/regen-fixtures.sh --oracle pdftex14027 \
  --profile initex-etex-eight-bit --validate-only
scripts/regen-fixtures.sh --oracle all --profile canonical --validate-only

if scripts/regen-fixtures.sh --oracle etex26 \
  --profile extended-eight-bit --validate-only >/dev/null 2>&1; then
  printf '%s\n' 'oracle regeneration accepted an incomplete e-TeX profile' >&2
  exit 1
fi
if scripts/regen-fixtures.sh --area tex82-oracle >/dev/null 2>&1; then
  printf '%s\n' 'oracle regeneration accepted the retired --area interface' >&2
  exit 1
fi
if scripts/regen-fixtures.sh --oracle etex26 \
  --profile compatibility+extended-eight-bit \
  --fixture tex82/command-transitions-v1 --validate-only >/dev/null 2>&1; then
  printf '%s\n' 'oracle regeneration accepted a fixture under the wrong engine' >&2
  exit 1
fi
if scripts/regen-fixtures.sh --oracle tex82 --profile initex-eight-bit \
  --fixture tex82/command-transitions-v1 --bootstrap-fixture \
  --validate-only >/dev/null 2>&1; then
  printf '%s\n' 'oracle regeneration accepted bootstrap without a live candidate' >&2
  exit 1
fi

printf '%s\n' 'oracle regeneration contract tests passed'
