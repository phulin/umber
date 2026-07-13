#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package_dir="${repo_root}/target/umber-wasm-package"

"${repo_root}/scripts/build-wasm-package.sh" "$package_dir"
node "${repo_root}/crates/umber-wasm/browser-tests/run.mjs" "$package_dir"
