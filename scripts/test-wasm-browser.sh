#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package_dir="${repo_root}/target/umber-wasm-package"
publisher="${repo_root}/tools/texlive-wasm-publish/target/debug/texlive-wasm-publish"

"${repo_root}/scripts/build-wasm-package.sh" "$package_dir"
cargo build -q --manifest-path "${repo_root}/tools/texlive-wasm-publish/Cargo.toml" --bin texlive-wasm-publish >"${repo_root}/target/umber-wasm-browser-publisher-build.log" 2>&1 || {
  cat "${repo_root}/target/umber-wasm-browser-publisher-build.log" >&2
  exit 1
}
node "${repo_root}/crates/umber-wasm/browser-tests/node-project.mjs" "$package_dir"
node "${repo_root}/crates/umber-wasm/browser-tests/run.mjs" "$package_dir" "$publisher"
