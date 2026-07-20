#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Local formatting and lint gate. Tests are run explicitly by callers so this
# script does not duplicate their test execution.

dprint check
if command -v biome >/dev/null 2>&1; then
  biome check \
    crates/umber-wasm/js \
    crates/umber-wasm/browser-tests \
    crates/umber-wasm/examples \
    crates/umber-wasm/package.json
else
  npx --yes @biomejs/biome@2.4.10 check \
    crates/umber-wasm/js \
    crates/umber-wasm/browser-tests \
    crates/umber-wasm/examples \
    crates/umber-wasm/package.json
fi
cargo fmt --all --check
CARGO_TARGET_DIR="${CLIPPY_TARGET_DIR:-target/clippy}" \
  cargo clippy --all-targets --quiet -- -D warnings

if [[ "${CHECK_BENCH:-0}" == 1 ]]; then
  scripts/check-node-width-budget.sh
fi
