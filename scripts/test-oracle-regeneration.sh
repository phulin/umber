#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

scripts/test-project-tex82-trip-command.py
scripts/test-etex26-extension-primitive-audit.sh
scripts/regen-fixtures.sh --oracle tex82 --profile initex-eight-bit --validate-only
scripts/regen-fixtures.sh --oracle tex82 --profile initex-eight-bit \
  --fixture tex82/command-transitions-v1 --validate-only
scripts/regen-fixtures.sh --oracle etex26 \
  --profile compatibility+extended-eight-bit --validate-only
scripts/regen-fixtures.sh --oracle pdftex14029 \
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

assert_document_trace_tree_publication() {
  local tmp_root
  tmp_root="$(mktemp -d)"

  mkdir -p "${tmp_root}/first-candidate"
  printf 'first\n' >"${tmp_root}/first-candidate/marker"
  scripts/build-tex82-document-traces.sh --test-publish-tree \
    "${tmp_root}/first-candidate" "${tmp_root}/published" \
    "${tmp_root}/previous"
  [[ "$(cat "${tmp_root}/published/marker")" == first ]]
  [[ ! -e "${tmp_root}/previous" ]]

  mkdir -p "${tmp_root}/replacement-candidate"
  printf 'replacement\n' >"${tmp_root}/replacement-candidate/marker"
  scripts/build-tex82-document-traces.sh --test-publish-tree \
    "${tmp_root}/replacement-candidate" "${tmp_root}/published" \
    "${tmp_root}/previous"
  [[ "$(cat "${tmp_root}/published/marker")" == replacement ]]
  [[ "$(cat "${tmp_root}/previous/marker")" == first ]]

  rm -rf "$tmp_root"
}

assert_document_trace_tree_publication

assert_source_manifest_drift_is_actionable() {
  local engine="$1"
  local profile="$2"
  local manifest_name="$3"
  local contract_row
  local pinned_digest
  local generated_digest
  local stale_digest
  local tmp_root
  local output
  tmp_root="$(mktemp -d)"
  mkdir -p "${tmp_root}/scripts" "${tmp_root}/tests"
  cp scripts/regen-fixtures.sh "${tmp_root}/scripts/"
  cp tests/oracle-regeneration-manifest.txt \
    tests/tex82-oracle-manifest.txt \
    tests/etex26-oracle-manifest.txt \
    tests/pdftex14029-oracle-manifest.txt \
    "${tmp_root}/tests/"
  contract_row="$(awk -v engine="$engine" \
    '$1 == "engine" && $2 == engine { print }' \
    "${tmp_root}/tests/oracle-regeneration-manifest.txt")"
  read -r _ _ contract_profile _ manifest_path pinned_digest _ \
    <<<"$contract_row"
  if [[ "$contract_profile" != "$profile" || \
        "$manifest_path" != "tests/${manifest_name}" ]]; then
    printf 'oracle regeneration test selector disagrees with the %s contract row\n' \
      "$engine" >&2
    rm -rf "$tmp_root"
    exit 1
  fi
  generated_digest="$(openssl dgst -sha256 -r \
    "${tmp_root}/${manifest_path}" | awk '{ print $1 }')"
  if [[ "$pinned_digest" != "$generated_digest" ]]; then
    printf 'oracle regeneration test copied a stale %s source-manifest pin: expected %s, generated %s\n' \
      "$engine" "$pinned_digest" "$generated_digest" >&2
    rm -rf "$tmp_root"
    exit 1
  fi
  if [[ "${generated_digest:0:1}" == 0 ]]; then
    stale_digest="1${generated_digest:1}"
  else
    stale_digest="0${generated_digest:1}"
  fi
  awk -v engine="$engine" -v stale_digest="$stale_digest" '
    $1 == "engine" && $2 == engine { $6 = stale_digest }
    { print }
  ' "${tmp_root}/tests/oracle-regeneration-manifest.txt" \
    >"${tmp_root}/tests/oracle-regeneration-manifest.txt.tmp"
  mv "${tmp_root}/tests/oracle-regeneration-manifest.txt.tmp" \
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
  tex82-oracle-manifest.txt
assert_source_manifest_drift_is_actionable \
  etex26 \
  compatibility+extended-eight-bit \
  etex26-oracle-manifest.txt
assert_source_manifest_drift_is_actionable \
  pdftex14029 \
  initex-etex-eight-bit \
  pdftex14029-oracle-manifest.txt

printf '%s\n' 'oracle regeneration contract tests passed'
