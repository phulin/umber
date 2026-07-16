#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

warn_missing_e2e_case() {
  local case_name="$1"
  local setup_hint="$2"
  shift 2

  local missing=()
  local path
  for path in "$@"; do
    if [[ ! -f "$path" ]]; then
      missing+=("${path#"$repo_root"/}")
    fi
  done

  if (( ${#missing[@]} == 0 )); then
    return
  fi

  printf 'check-and-test: warning: %s e2e conformance will be skipped; missing:' "$case_name" >&2
  printf ' %s' "${missing[@]}" >&2
  printf '\ncheck-and-test: warning: %s\n' "$setup_hint" >&2
}

warn_missing_e2e_case \
  "Story" \
  "run scripts/setup-conformance-tests.sh to install the Story/Gentle corpus" \
  "$repo_root/third_party/corpus/story.tex" \
  "$repo_root/third_party/corpus/plain.tex" \
  "$repo_root/third_party/hyphen/hyphen.tex" \
  "$repo_root/tests/corpus/e2e/story.expected.dvi"
warn_missing_e2e_case \
  "Gentle" \
  "run scripts/setup-conformance-tests.sh to install the Story/Gentle corpus" \
  "$repo_root/third_party/corpus/gentle.tex" \
  "$repo_root/third_party/corpus/plain.tex" \
  "$repo_root/third_party/hyphen/hyphen.tex" \
  "$repo_root/tests/corpus/e2e/gentle.expected.dvi"
warn_missing_e2e_case \
  "TRIP" \
  "run scripts/trip.sh fetch to install the TRIP/e-TRIP corpus" \
  "$repo_root/third_party/trip/trip.tex" \
  "$repo_root/third_party/trip/trip.tfm" \
  "$repo_root/tests/corpus/e2e/trip.expected.dvi"
warn_missing_e2e_case \
  "e-TRIP" \
  "run scripts/trip.sh fetch to install the TRIP/e-TRIP corpus" \
  "$repo_root/third_party/trip/etrip.tex" \
  "$repo_root/third_party/trip/trip.tfm" \
  "$repo_root/tests/corpus/e2e/etrip.expected.dvi"

scripts/test-publish-texlive-r2.sh

cargo test --workspace --tests --quiet &
test_pid=$!
scripts/check.sh &
check_pid=$!

if wait "$test_pid"; then
  test_status=0
else
  test_status=$?
fi

if wait "$check_pid"; then
  check_status=0
else
  check_status=$?
fi

if (( test_status != 0 )); then
  exit "$test_status"
fi
exit "$check_status"
