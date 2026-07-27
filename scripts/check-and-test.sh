#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Fail-fast preflight for the byte-exact end-to-end DVI conformance gates.
#
# The oracle list is read from `.gitignore` rather than restated here: that is
# the same single source `assets::conformance_gate_registry_matches_gitignore`
# binds the Rust gate registry to, so a new gate cannot leave this preflight
# stale. Absence is only reported here; the gates themselves enforce it by
# failing (see the End-to-End Conformance Gate Contract in
# docs/testing_infrastructure.md).
warn_missing_e2e_oracles() {
  local missing=()
  local entry
  while read -r entry; do
    [[ -f "${repo_root}${entry}" ]] || missing+=("${entry#/}")
  done < <(grep -E '^/tests/corpus/e2e/.+\.expected\.dvi$' .gitignore || true)

  if (( ${#missing[@]} == 0 )); then
    return
  fi

  printf 'check-and-test: warning: end-to-end DVI conformance oracles are absent:' >&2
  printf ' %s' "${missing[@]}" >&2
  printf '\ncheck-and-test: warning: those gates will FAIL, not skip; run scripts/setup-conformance-tests.sh\n' >&2
}

warn_missing_e2e_oracles

scripts/test-publish-texlive-r2.sh

# The suite is run through `run-native-tests.py` rather than a bare
# `cargo test --tests`: that command selects the workspace's default members,
# which left the `bib-*` crates, `umber-interrupt`, `refexec`, and
# `profile-analyzer` executed by no routine gate (`umber2-johp.211`). The
# script selects `--workspace` minus a declared, verified exclusion list and
# ends in a `VERDICT:` line naming what actually ran.
scripts/run-native-tests.py &
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
