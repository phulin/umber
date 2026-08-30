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
#
# Naming gates on the command line runs exactly those gates, with byte-identical
# commands. This exists so nobody has to retype a gate's invocation: a hand
# written `cargo clippy -p <crate>` omits the `-D warnings` that gives the
# workspace lint policy its teeth, exits 0 on a real violation, and makes a
# "clippy clean" report mean nothing. `scripts/check.sh clippy` cannot diverge
# from the gate because it is the gate.

all_gates=(dprint biome rustfmt clippy node-width-budget)
selected_gates=()

usage() {
  cat >&2 <<EOF
usage: ${BASH_SOURCE[0]##*/} [gate ...]

Runs the repository format and lint gates. With no arguments every gate runs.
Naming gates runs exactly those, with the same commands the full run uses.

gates: ${all_gates[*]}
EOF
}

for requested in "$@"; do
  case " ${all_gates[*]} " in
    *" $requested "*) selected_gates+=("$requested") ;;
    *)
      printf 'check.sh: unknown gate %q\n\n' "$requested" >&2
      usage
      exit 2
      ;;
  esac
done

# The node-width budget is opt-in in a full run because it is slow; naming it
# explicitly is itself the opt-in.
if ((${#selected_gates[@]} == 0)); then
  selected_gates=(dprint biome rustfmt clippy)
  if [[ "${CHECK_BENCH:-0}" == 1 ]]; then
    selected_gates+=(node-width-budget)
  fi
fi

failed_gates=()
blocked_gates=()
ran_gates=0

gate() {
  local name="$1"
  shift
  case " ${selected_gates[*]} " in
    *" $name "*) ;;
    *) return 0 ;;
  esac
  ran_gates=$((ran_gates + 1))
  printf '\n=== check.sh gate: %s\n' "$name"
  local status=0
  "$@" || status=$?
  if ((status == 4)); then
    blocked_gates+=("$name (exit $status)")
  elif ((status != 0)); then
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
  # One clippy invocation lints one feature resolution, and Cargo unifies
  # features across everything the invocation selects, so no single command can
  # lint both the whole-workspace test union and the resolution the shipped
  # binary is built in. The gate therefore runs a declared set of passes;
  # `scripts/check-lint-passes.py` documents and verifies what each one covers.
  # Its own guards are self-tested first, because a coverage check that cannot
  # fail is worth less than no check at all.
  python3 scripts/test-check-lint-passes.py || return 1
  CARGO_TARGET_DIR="${CLIPPY_TARGET_DIR:-target/clippy}" \
    python3 scripts/check-lint-passes.py
}

gate dprint run_dprint
gate biome run_biome
gate rustfmt cargo fmt --all --check
gate clippy run_clippy
gate node-width-budget scripts/check-node-width-budget.sh

if ((${#failed_gates[@]} == 0 && ${#blocked_gates[@]} == 0)); then
  printf '\ncheck.sh: all %d gates passed.\n' "$ran_gates"
  exit 0
fi

if ((${#failed_gates[@]} > 0)); then
  printf '\ncheck.sh: %d of %d gates FAILED:\n' "${#failed_gates[@]}" "$ran_gates" >&2
  printf '  - %s\n' "${failed_gates[@]}" >&2
  if ((${#blocked_gates[@]} > 0)); then
    printf 'check.sh: %d additional gates BLOCKED:\n' "${#blocked_gates[@]}" >&2
    printf '  - %s\n' "${blocked_gates[@]}" >&2
  fi
  exit 1
fi

printf '\ncheck.sh: %d of %d gates BLOCKED:\n' "${#blocked_gates[@]}" "$ran_gates" >&2
printf '  - %s\n' "${blocked_gates[@]}" >&2
exit 4
