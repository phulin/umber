#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-node-width-budget.XXXXXX")"
trap 'rm -rf "$work_root"' EXIT

make_fixture() {
  local fixture="$1"
  mkdir -p "$fixture/scripts" "$fixture/benchmarks/tex-typeset" "$fixture/bin"
  cp "$repo_root/scripts/check-node-width-budget.sh" "$fixture/scripts/"
  cp "$repo_root/scripts/check.sh" "$fixture/scripts/"
  cp "$repo_root/benchmarks/tex-typeset/node-width-budgets.json" \
    "$fixture/benchmarks/tex-typeset/"
  : >"$fixture/benchmarks/tex-typeset/Cargo.toml"

  cat >"$fixture/bin/rustc" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
cat <<EOF
rustc 1.93.0
binary: rustc
commit-hash: test
commit-date: 2026-08-01
host: ${MOCK_RUSTC_HOST:-aarch64-apple-darwin}
release: ${MOCK_RUSTC_RELEASE:-1.93.0}
LLVM version: test
EOF
SH
  chmod +x "$fixture/bin/rustc"

  cat >"$fixture/bin/cargo" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf 'invoked\n' >"$MOCK_CARGO_LOG"
for row in same_font_64 same_font_4096 mixed_4096; do
  result="$CARGO_TARGET_DIR/criterion/hpack_widths/$row/new"
  mkdir -p "$result"
  printf '{"mean":{"point_estimate":1.0}}\n' >"$result/estimates.json"
done
SH
  chmod +x "$fixture/bin/cargo"
}

run_fixture() {
  local fixture="$1"
  shift
  PATH="$fixture/bin:$PATH" \
    CARGO_TARGET_DIR="$fixture/target" \
    MOCK_CARGO_LOG="$fixture/cargo.log" \
    "$@" "$fixture/scripts/check.sh" node-width-budget
}

match="$work_root/match"
make_fixture "$match"
run_fixture "$match" >"$match/output"
grep -Fq '"status": "applicable"' "$match/output"
grep -Fq '"status": "pass"' "$match/output"
grep -Fq 'check.sh: all 1 gates passed.' "$match/output"
test -f "$match/cargo.log"

mismatch="$work_root/mismatch"
make_fixture "$mismatch"
set +e
MOCK_RUSTC_HOST=x86_64-unknown-linux-gnu \
  run_fixture "$mismatch" >"$mismatch/output" 2>&1
status=$?
set -e
test "$status" -eq 4
grep -Fq '"status": "unsupported"' "$mismatch/output"
grep -Fq '"gating": false' "$mismatch/output"
grep -Fq 'check.sh: 1 of 1 gates BLOCKED:' "$mismatch/output"
if grep -Fq 'gates FAILED' "$mismatch/output"; then
  printf 'unsupported host was reported as a failure\n' >&2
  exit 1
fi
test ! -e "$mismatch/cargo.log"

malformed="$work_root/malformed"
make_fixture "$malformed"
python3 - "$malformed/benchmarks/tex-typeset/node-width-budgets.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
baseline = json.loads(path.read_text())
baseline["benchmarks"]["renamed_row"] = baseline["benchmarks"].pop("same_font_64")
path.write_text(json.dumps(baseline))
PY
set +e
run_fixture "$malformed" >"$malformed/output" 2>&1
status=$?
set -e
test "$status" -eq 1
grep -Fq '"status": "invalid_baseline"' "$malformed/output"
grep -Fq 'benchmark rows must be exactly' "$malformed/output"
grep -Fq 'check.sh: 1 of 1 gates FAILED:' "$malformed/output"
test ! -e "$malformed/cargo.log"

printf 'node width budget script tests passed\n'
