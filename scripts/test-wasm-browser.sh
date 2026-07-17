#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package_dir="${repo_root}/target/umber-wasm-package"
native_bin="${repo_root}/target/debug/umber"

"${repo_root}/scripts/build-wasm-package.sh" "$package_dir"
cargo build -q -p umber --bin umber
node "${repo_root}/crates/umber-wasm/browser-tests/node-project.mjs" "$package_dir"
node "${repo_root}/crates/umber-wasm/browser-tests/run.mjs" "$package_dir" "$native_bin"
