#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Preflight for the byte-exact end-to-end DVI conformance gates.
#
# The oracle list is read from `.gitignore` rather than restated here: that is
# the same single source `assets::conformance_gate_registry_matches_gitignore`
# binds the Rust gate registry to, so a new gate cannot leave this preflight
# stale. Absence is only reported here; the gates themselves enforce it by
# failing (see the End-to-End Conformance Gate Contract in
# docs/testing_infrastructure.md).
missing_oracles=()
find_missing_e2e_oracles() {
  local entry
  while read -r entry; do
    [[ -f "${repo_root}${entry}" ]] || missing_oracles+=("${entry#/}")
  done < <(grep -E '^/tests/corpus/e2e/.+\.expected\.dvi$' .gitignore || true)
}

find_missing_e2e_oracles
failed=()
blocked=()
partial=()
passed=()

record_status() {
  local name="$1" status="$2"
  if (( status == 0 )); then
    passed+=("$name")
  elif (( status == 4 )); then
    blocked+=("$name (exit $status)")
  else
    failed+=("$name (exit $status)")
  fi
}

run_stage() {
  local name="$1" status=0
  shift
  "$@" || status=$?
  record_status "$name" "$status"
  return 0
}

if (( ${#missing_oracles[@]} > 0 )); then
  printf 'check-and-test: missing end-to-end DVI conformance oracles:' >&2
  printf ' %s' "${missing_oracles[@]}" >&2
  printf '\ncheck-and-test: run python3 scripts/provision.py worktree . in this checkout\n' >&2
  if [[ "${UMBER_CONFORMANCE_ORACLES:-}" == optional ]]; then
    partial+=("DVI conformance (${#missing_oracles[@]} absent oracles; explicit opt-out)")
  else
    blocked+=("DVI conformance (${#missing_oracles[@]} absent oracles)")
  fi
fi

run_stage publish-contract scripts/test-publish-texlive-r2.sh
run_stage asset-contract python3 scripts/test-native-test-assets.py
run_stage gate-contract scripts/test-gate-verdicts.sh

# `cargo test --tests` is the whole routine suite: `default-members` lists
# every host-testable member, and `default_members_cover_every_host_testable_crate`
# in `test-support` fails if that list ever drifts from the workspace again
# (`umber2-johp.211`). Compile that suite before starting the independent
# clippy build: launching both cold Cargo workloads together overwhelms smaller
# development hosts and makes a fresh worktree slower rather than faster.
prebuild_status=0
cargo test --quiet --tests --no-run || prebuild_status=$?
record_status native-prebuild "$prebuild_status"

if (( prebuild_status == 0 )); then
  # Libtest discovery is authoritative for ignored host cases. It reads the
  # already-built executables and rejects newly unowned optional suites.
  run_stage rust-suite-inventory python3 scripts/check-selected-rust-suites.py
  python3 scripts/run-umber-guarded.py \
    --timeout-seconds 1800 --max-rss-mib 6144 --term-grace-seconds 5 -- \
    cargo test --quiet --tests &
  test_pid=$!
  scripts/check.sh &
  check_pid=$!
  run_stage native-tests wait "$test_pid"
  run_stage quality wait "$check_pid"
else
  blocked+=("rust-suite-inventory (prebuild failed)")
  blocked+=("native-tests (prebuild failed)")
  run_stage quality scripts/check.sh
fi

printf '\ncheck-and-test: stages: %d passed, %d failed, %d blocked; %d coverage reductions\n' \
  "${#passed[@]}" "${#failed[@]}" "${#blocked[@]}" "${#partial[@]}"
if (( ${#failed[@]} > 0 )); then
  printf '  FAILED: %s\n' "${failed[@]}" >&2
fi
if (( ${#blocked[@]} > 0 )); then
  printf '  BLOCKED: %s\n' "${blocked[@]}" >&2
fi
if (( ${#partial[@]} > 0 )); then
  printf '  PARTIAL: %s\n' "${partial[@]}" >&2
fi
if (( ${#failed[@]} > 0 )); then
  printf 'check-and-test: VERDICT: FAIL\n' >&2
  exit 1
elif (( ${#blocked[@]} > 0 )); then
  printf 'check-and-test: VERDICT: BLOCKED\n' >&2
  exit 4
elif (( ${#partial[@]} > 0 )); then
  printf 'check-and-test: VERDICT: PARTIAL\n'
else
  printf 'check-and-test: VERDICT: PASS\n'
fi
