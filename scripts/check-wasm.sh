#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Explicit gate for the WebAssembly binding and the authored browser package.
#
# The routine `cargo test --tests` suite deliberately omits `umber-wasm`: its tests
# are `#[wasm_bindgen_test]`, which registers no test on a host target, so
# selecting it there builds a cdylib and runs exactly zero tests. This check is
# where they run for real, and it needs wasm-pack, a headless browser, and Node,
# which the routine gate must not depend on.
#
# Each step is named, and an absent browser or wasm-pack reports BLOCKED rather
# than passing or aborting the rest of the run.

# shellcheck source=scripts/optional-check-runner.sh
source "$repo_root/scripts/optional-check-runner.sh"

OPTIONAL_CHECK_ARGS="$*" optional_check_begin check-wasm.sh \
  wasm-check biome node-unit wasm-bindgen browser-package npm-pack

biome_check() {
  local biome_cmd=(npx --yes @biomejs/biome@2.4.10)
  if command -v biome >/dev/null 2>&1; then
    biome_cmd=(biome)
  fi
  "${biome_cmd[@]}" check \
    crates/umber-wasm/js \
    crates/umber-wasm/browser-tests \
    crates/umber-wasm/examples \
    crates/umber-wasm/package.json
}

node_unit() {
  node --test crates/umber-wasm/js/*.test.js
}

npm_pack() {
  (
    cd target/umber-wasm-package || exit 1
    npm pack --dry-run --json >/dev/null
  )
}

optional_check_step wasm-check cargo check -p umber-wasm --target wasm32-unknown-unknown
optional_check_step_requiring npx biome biome_check
optional_check_step_requiring node node-unit node_unit
optional_check_step_requiring "wasm-pack firefox" wasm-bindgen \
  wasm-pack test --headless --firefox crates/umber-wasm
# Both of these consume `target/umber-wasm-package`, which
# `scripts/build-wasm-package.sh` builds with wasm-pack, so an absent wasm-pack
# blocks them rather than failing them: nothing was measured either way, and
# calling that a failure would bury the one real signal in noise.
optional_check_step_requiring "node wasm-pack" browser-package scripts/test-wasm-browser.sh
optional_check_step_requiring "npm wasm-pack" npm-pack npm_pack

optional_check_finish
