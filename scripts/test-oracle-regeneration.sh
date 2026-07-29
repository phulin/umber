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
assert_source_manifest_drift_is_actionable() {
  local engine="$1"
  local profile="$2"
  local manifest_name="$3"
  local stale_digest="$4"
  local generated_digest="$5"
  local tmp_root
  local output
  tmp_root="$(mktemp -d)"
  mkdir -p "${tmp_root}/scripts" "${tmp_root}/tests"
  cp scripts/regen-fixtures.sh "${tmp_root}/scripts/"
  cp tests/oracle-regeneration-manifest.txt \
    tests/tex82-oracle-manifest.txt \
    tests/etex26-oracle-manifest.txt \
    tests/pdftex14027-oracle-manifest.txt \
    "${tmp_root}/tests/"
  sed -i "s/${generated_digest}/${stale_digest}/" \
    "${tmp_root}/tests/oracle-regeneration-manifest.txt"
  if output="$(
    cd "$tmp_root"
    scripts/regen-fixtures.sh --oracle "$engine" --profile "$profile" \
      --validate-only 2>&1
  )"; then
    printf 'oracle regeneration accepted stale %s source-manifest identity\n' \
      "$engine" >&2
    rm -rf "$tmp_root"
    exit 1
  fi
  printf '%s\n' "$output" | grep -Fqx \
    "regen-fixtures: ${engine} source manifest identity drift: tests/${manifest_name}: expected ${stale_digest}, generated ${generated_digest}" || {
    printf 'oracle regeneration emitted an unactionable %s source-manifest diagnostic:\n%s\n' \
      "$engine" "$output" >&2
    rm -rf "$tmp_root"
    exit 1
  }
  rm -rf "$tmp_root"
}

assert_source_manifest_drift_is_actionable \
  tex82 \
  initex-eight-bit \
  tex82-oracle-manifest.txt \
  d8bd0fa161d2fa1b0d9634198fff5b8f20c9e9986b7be363134686965751cff8 \
  845ad8acfa69254ba120f1a9f666e2a80a2dd991e5b7f0215d2d2b1c9dc1a266
assert_source_manifest_drift_is_actionable \
  etex26 \
  compatibility+extended-eight-bit \
  etex26-oracle-manifest.txt \
  abb05cc5bef25608574fc309a2f3253c19816c3ac8dfb4b3c94721479eb82a1e \
  77b06296b862f298afd78ca6f5dac2a0a9e8dc27f095bebf03bf214ab07041f9
