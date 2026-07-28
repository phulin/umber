#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# The browser/package build needs Node and wasm-pack. Missing prerequisites are
# recorded as BLOCKED rather than as a successful skipped comparison.
source "$repo_root/scripts/tier-runner.sh"

TIER_ARGS="$*" tier_begin check-latex-wasm.sh latex-wasm-parity
tier_step_requiring "awk cargo mktemp node wasm-pack" latex-wasm-parity \
  "$repo_root/scripts/run-latex-wasm.sh"
tier_finish
