#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo test --workspace --tests &
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
