#!/usr/bin/env bash
set -euo pipefail

CASES="${PROPTEST_CASES:-10000}"

echo "Running effectful rollback/commit fuzz with PROPTEST_CASES=${CASES}"
PROPTEST_CASES="${CASES}" cargo test -p umber --test effectful_replay
