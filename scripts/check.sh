#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Local formatting and lint gate. Tests are run explicitly by callers so this
# script does not duplicate their test execution.

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
