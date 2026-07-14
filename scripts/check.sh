#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Local formatting and lint gate. Tests are run explicitly by callers so this
# script does not duplicate their test execution.

cargo fmt --all --check
CARGO_TARGET_DIR="${CLIPPY_TARGET_DIR:-target/clippy}" \
  cargo clippy --workspace --all-targets -- -D warnings

if [[ "${CHECK_BENCH:-0}" == 1 ]]; then
  scripts/check-node-width-budget.sh
fi
