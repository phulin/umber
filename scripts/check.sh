#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Local formatting and lint gate. Tests are run explicitly by callers so this
# script does not duplicate their test execution.
#
# Every gate runs on every invocation and failures are aggregated, so a red
# formatter can never hide a red linter: under `set -e` a failing `dprint check`
# aborted the script before clippy ever ran, and the run reported only the first
# failure.

failed_gates=()
ran_gates=0

gate() {
  local name="$1"
  shift
  ran_gates=$((ran_gates + 1))
  printf '\n=== check.sh gate: %s\n' "$name"
  local status=0
  "$@" || status=$?
  if ((status != 0)); then
    failed_gates+=("$name (exit $status)")
  fi
  return 0
}

require_tool() {
  local name="$1"
  local hint="$2"
  if command -v "$name" >/dev/null 2>&1; then
    return 0
  fi
  printf 'check.sh: %s is not installed; %s\n' "$name" "$hint" >&2
  return 1
}

run_dprint() {
  require_tool dprint "install it with: npm install --global dprint@0.55.2" || return 1
  dprint check
}

run_biome() {
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

run_clippy() {
  CARGO_TARGET_DIR="${CLIPPY_TARGET_DIR:-target/clippy}" \
    cargo clippy --all-targets --quiet -- -D warnings
}

gate dprint run_dprint
gate biome run_biome
gate rustfmt cargo fmt --all --check
gate clippy run_clippy

if [[ "${CHECK_BENCH:-0}" == 1 ]]; then
  gate node-width-budget scripts/check-node-width-budget.sh
fi

if ((${#failed_gates[@]} == 0)); then
  printf '\ncheck.sh: all %d gates passed.\n' "$ran_gates"
  exit 0
fi

printf '\ncheck.sh: %d of %d gates FAILED:\n' "${#failed_gates[@]}" "$ran_gates" >&2
printf '  - %s\n' "${failed_gates[@]}" >&2
exit 1
