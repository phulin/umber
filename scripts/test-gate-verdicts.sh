#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
fixture="$scratch/repo"
mkdir -p "$fixture/scripts" "$scratch/bin" "$fixture/tests/corpus/e2e"
cp "$repo_root/scripts/check-and-test.sh" "$repo_root/scripts/check.sh" \
  "$repo_root/scripts/optional-check-runner.sh" "$fixture/scripts/"
printf '/tests/corpus/e2e/story.expected.dvi\n' > "$fixture/.gitignore"
printf 'oracle\n' > "$fixture/tests/corpus/e2e/story.expected.dvi"

cat > "$scratch/bin/cargo" <<'SH'
#!/usr/bin/env bash
if [[ "$1" == fmt ]]; then
  exit "${RUSTFMT_STATUS:-0}"
fi
if [[ "$*" == *--no-run* ]]; then
  printf 'prebuild\n' >> "$STAGE_TRACE"
  exit "${PREBUILD_STATUS:-0}"
fi
printf 'native-tests\n' >> "$STAGE_TRACE"
exit "${NATIVE_STATUS:-0}"
SH
cat > "$fixture/scripts/run-umber-guarded.py" <<'SH'
import subprocess
import sys
raise SystemExit(subprocess.call(sys.argv[sys.argv.index("--") + 1:]))
SH
cat > "$fixture/scripts/test-publish-texlive-r2.sh" <<'SH'
#!/usr/bin/env bash
printf 'publish\n' >> "$STAGE_TRACE"
exit "${PUBLISH_STATUS:-0}"
SH
cat > "$fixture/scripts/test-native-test-assets.py" <<'SH'
import os
with open(os.environ["STAGE_TRACE"], "a") as trace:
    trace.write("assets\n")
raise SystemExit(int(os.environ.get("ASSET_STATUS", "0")))
SH
cat > "$fixture/scripts/test-gate-verdicts.sh" <<'SH'
#!/usr/bin/env bash
printf 'gate-contract\n' >> "$STAGE_TRACE"
exit "${CONTRACT_STATUS:-0}"
SH
cat > "$fixture/scripts/check.sh" <<'SH'
#!/usr/bin/env bash
printf 'quality\n' >> "$STAGE_TRACE"
exit "${QUALITY_STATUS:-0}"
SH
chmod +x "$scratch/bin/cargo" "$fixture/scripts/"*.sh \
  "$fixture/scripts/"*.py

cat > "$scratch/bin/dprint" <<'SH'
#!/usr/bin/env bash
exit "${DPRINT_STATUS:-0}"
SH
chmod +x "$scratch/bin/dprint"

assert_case() {
  local name="$1" expected_status="$2" expected_verdict="$3"
  shift 3
  : > "$scratch/trace"
  local status=0
  env PATH="$scratch/bin:$PATH" STAGE_TRACE="$scratch/trace" "$@" \
    "$fixture/scripts/check-and-test.sh" > "$scratch/output" 2>&1 || status=$?
  if (( status != expected_status )) ||
    ! grep -Fq "check-and-test: VERDICT: $expected_verdict" "$scratch/output"; then
    printf 'gate verdict case %s: wanted %s/%s, got %s:\n' \
      "$name" "$expected_status" "$expected_verdict" "$status" >&2
    cat "$scratch/output" >&2
    exit 1
  fi
}

assert_case pass 0 PASS
grep -Fqx prebuild "$scratch/trace"
grep -Fqx native-tests "$scratch/trace"
grep -Fqx quality "$scratch/trace"

assert_case combined-fail 1 FAIL \
  PUBLISH_STATUS=3 ASSET_STATUS=5 CONTRACT_STATUS=6 PREBUILD_STATUS=7 QUALITY_STATUS=8
for expected in publish-contract asset-contract gate-contract native-prebuild \
  'native-tests (prebuild failed)' quality; do
  grep -Fq "$expected" "$scratch/output"
done
if grep -Fqx native-tests "$scratch/trace"; then
  printf 'native tests ran after a failed prebuild\n' >&2
  exit 1
fi

assert_case fail-and-blocked 1 FAIL NATIVE_STATUS=9 QUALITY_STATUS=4
grep -Fq 'FAILED: native-tests (exit 9)' "$scratch/output"
grep -Fq 'BLOCKED: quality (exit 4)' "$scratch/output"

rm "$fixture/tests/corpus/e2e/story.expected.dvi"
assert_case missing-required 4 BLOCKED
grep -Fq 'BLOCKED: DVI conformance (1 absent oracles)' "$scratch/output"
assert_case missing-optional 0 PARTIAL UMBER_CONFORMANCE_ORACLES=optional
grep -Fq 'PARTIAL: DVI conformance (1 absent oracles; explicit opt-out)' "$scratch/output"
assert_case missing-optional-and-fail 1 FAIL \
  UMBER_CONFORMANCE_ORACLES=optional NATIVE_STATUS=9
grep -Fq 'PARTIAL: DVI conformance' "$scratch/output"

# The shared optional runner must retain every cause, including a command's
# own BLOCKED status, missing tools, and unselected steps.
runner="$repo_root/scripts/optional-check-runner.sh"
status=0
RUNNER="$runner" bash -c '
  source "$RUNNER"
  OPTIONAL_CHECK_ARGS="failed missing blocked" optional_check_begin fixture \
    failed missing blocked unselected
  optional_check_step failed bash -c "exit 7"
  optional_check_step_requiring "nonexistent_umber_gate_tool" missing true
  optional_check_step blocked bash -c "exit 4"
  optional_check_finish
' > "$scratch/output" 2>&1 || status=$?
[[ "$status" == 1 ]]
grep -Fq 'VERDICT: FAIL' "$scratch/output"
grep -Fq '1 failed: failed; 2 blocked: missing blocked' "$scratch/output"
grep -Fq '1 not selected' "$scratch/output"

status=0
RUNNER="$runner" bash -c '
  source "$RUNNER"
  OPTIONAL_CHECK_ARGS="ok" optional_check_begin fixture ok other
  optional_check_step ok true
  optional_check_finish
' > "$scratch/output" 2>&1 || status=$?
[[ "$status" == 0 ]]
grep -Fq 'VERDICT: PARTIAL' "$scratch/output"

status=0
RUNNER="$runner" bash -c '
  source "$RUNNER"
  optional_check_begin fixture blocked
  optional_check_step blocked bash -c "exit 4"
  optional_check_finish
' > "$scratch/output" 2>&1 || status=$?
[[ "$status" == 4 ]]
grep -Fq 'VERDICT: BLOCKED' "$scratch/output"

status=0
RUNNER="$runner" bash -c '
  source "$RUNNER"
  OPTIONAL_CHECK_ARGS="unknown" optional_check_begin fixture known
' > "$scratch/output" 2>&1 || status=$?
[[ "$status" == 2 ]]
grep -Fq 'unknown step' "$scratch/output"

quality_fixture="$scratch/quality"
mkdir -p "$quality_fixture/scripts"
cp "$repo_root/scripts/check.sh" "$quality_fixture/scripts/"
status=0
env PATH="$scratch/bin:$PATH" DPRINT_STATUS=3 RUSTFMT_STATUS=4 \
  "$quality_fixture/scripts/check.sh" dprint rustfmt \
  > "$scratch/output" 2>&1 || status=$?
[[ "$status" == 1 ]]
grep -Fq '1 of 2 gates FAILED:' "$scratch/output"
grep -Fq '1 additional gates BLOCKED:' "$scratch/output"
grep -Fq 'dprint (exit 3)' "$scratch/output"
grep -Fq 'rustfmt (exit 4)' "$scratch/output"

sparse_bin="$scratch/sparse-bin"
mkdir -p "$sparse_bin"
ln -s "$(command -v bash)" "$sparse_bin/bash"
ln -s "$(command -v dirname)" "$sparse_bin/dirname"
status=0
/usr/bin/env PATH="$sparse_bin" "$quality_fixture/scripts/check.sh" dprint \
  > "$scratch/output" 2>&1 || status=$?
[[ "$status" == 4 ]]
grep -Fq 'dprint is not installed' "$scratch/output"
grep -Fq '1 of 1 gates BLOCKED:' "$scratch/output"

# Both names for the validation-only oracle step select the same public check.
tool_fixture="$scratch/tools"
mkdir -p "$tool_fixture/scripts"
cp "$repo_root/scripts/check-tools.sh" "$repo_root/scripts/optional-check-runner.sh" \
  "$tool_fixture/scripts/"
cat > "$tool_fixture/scripts/test-oracle-regeneration.sh" <<'SH'
#!/usr/bin/env bash
printf 'oracle-contract\n' >> "$STAGE_TRACE"
SH
chmod +x "$tool_fixture/scripts/"*.sh
for selector in oracle-contract oracle-regeneration; do
  : > "$scratch/trace"
  STAGE_TRACE="$scratch/trace" \
    "$tool_fixture/scripts/check-tools.sh" "$selector" \
    > "$scratch/output" 2>&1
  grep -Fqx oracle-contract "$scratch/trace"
  grep -Fq 'check-tools.sh: VERDICT: PARTIAL' "$scratch/output"
done

python3 "$repo_root/scripts/test-script-suite-inventory.py"

printf 'gate verdict contracts: PASS\n'
