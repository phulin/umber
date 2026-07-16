#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo check -p umber-wasm --target wasm32-unknown-unknown

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
node --test crates/umber-wasm/js/*.test.js
wasm-pack test --headless --firefox crates/umber-wasm
scripts/test-wasm-browser.sh
(
  cd target/umber-wasm-package
  npm pack --dry-run --json >/dev/null
)
