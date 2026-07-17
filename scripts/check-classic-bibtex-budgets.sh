#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# These release-only tests own the fixed classic compilation/cache and native
# session timing budgets. The wasm-bindgen browser suite owns the matching
# WASM-session timing and retained-cache budget; the broader JS/package lint
# gate remains scripts/check-wasm.sh.
cargo test --release -q -p bib-bst classic_compilation_and_cache_performance_budgets -- --ignored
cargo test --release -q -p bib-engine --test it classic_native_session_performance_budget -- --ignored
wasm-pack test --headless --firefox crates/umber-wasm
