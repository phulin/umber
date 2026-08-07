#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/umber-arxiv-census-test.XXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM

mkdir -p "$work/archive-input/ok" "$work/archive-input/finalfail" "$work/archive-input/enginefail"
mkdir -p "$work/archives" "$work/corpus"
mkdir -p "$work/distribution" "$work/texmf"
printf 'id\tcategories\nok\ttest\nfinalfail\ttest\nenginefail\ttest\n' >"$work/sample.tsv"
for paper in ok finalfail enginefail; do
  printf '\\documentclass{article}\n' >"$work/archive-input/$paper/main.tex"
  tar -czf "$work/archives/$paper.src" -C "$work/archive-input/$paper" main.tex
done
printf 'format\n' >"$work/format.fmt"
printf '{}\n' >"$work/distribution/manifest.json"

cat >"$work/fake-umber" <<'EOF'
#!/bin/sh
set -eu
: "${FAKE_UMBER_COUNT:?}"
count=0
test ! -f "$FAKE_UMBER_COUNT" || count=$(cat "$FAKE_UMBER_COUNT")
count=$((count + 1))
printf '%s\n' "$count" >"$FAKE_UMBER_COUNT"
pdf=
inputs=
previous=
for argument in "$@"; do
  test "$previous" != pdf || pdf=$argument
  test "$previous" != inputs || inputs=$argument
  case "$argument" in
    --pdf) previous=pdf ;;
    --input-records-out) previous=inputs ;;
    *) previous= ;;
  esac
done
main=$argument
printf 'generated\n' >"$(dirname "$main")/document.aux"
case "$main" in
  *enginefail*)
    test "${UMBER_CAUSAL_DIAGNOSTIC:-}" = 1
    echo 'CAUSAL_DIAGNOSTIC schema=1 cause_sha256=abc source_sha256=def bytes=9..11 line=1 column=10 cause_kind=command-error input_frames=11 input_tail="macro-body,macro-argument,source" group_depth=2 group_tail="simple@1,hbox@0"' >&2
    echo 'invalid parameter token' >&2
    exit 1
    ;;
esac
echo RESOURCE_ENGINE_ACCEPTED >&2
echo 'RESOURCE_TELEMETRY cold_starts=1 suspensions=2 local_step_retries=2 replayed_delivered_tokens=3 replayed_dispatches=3 cumulative_fuel=4 resource_wait_ns=5 engine_ns=6' >&2
case "$main" in
  *finalfail*) echo 'action type missing' >&2; exit 1 ;;
esac
printf 'pdf\n' >"$pdf"
printf '1\tinput\n' >"$inputs"
EOF
chmod +x "$work/fake-umber"

run_census() {
  FAKE_UMBER_COUNT="$work/count" \
  UMBER_ARXIV_SAMPLE="$work/sample.tsv" \
  UMBER_ARXIV_CORPUS="$work/corpus" \
  UMBER_ARXIV_ARCHIVES="$work/archives" \
  UMBER_ARXIV_FORMAT="$work/format.fmt" \
  UMBER_ARXIV_DISTRIBUTION="$work/distribution" \
  UMBER_ARXIV_BINARY="$work/fake-umber" \
  UMBER_ARXIV_RESULTS="$work/results" \
  UMBER_ARXIV_TEXMF="$work/texmf" \
  UMBER_ARXIV_LIMIT=3 \
  UMBER_ARXIV_TIMEOUT_SECONDS=10 \
  UMBER_ARXIV_MAX_RSS_MIB=128 \
  UMBER_ARXIV_ENGINE_FUEL=1000 \
  "$root/scripts/run-stepwise-arxiv-census.sh"
}

run_census
test "$(cat "$work/count")" -eq 3
test ! -e "$work/corpus/ok/document.aux"
awk -F '\t' '$1 == "ok" { exit !($2 == "accepted" && $3 == "complete") }' "$work/results/summary.tsv"
awk -F '\t' '$1 == "finalfail" { exit !($2 == "accepted" && $3 == "failed") }' "$work/results/summary.tsv"
awk -F '\t' '$1 == "enginefail" { exit !($2 == "failed" && $3 == "not-run") }' "$work/results/summary.tsv"
test "$(grep -c '^CAUSAL_DIAGNOSTIC ' "$work/results/enginefail.engine.log")" -eq 1
test "$(wc -c <"$work/results/enginefail.engine.log")" -lt 1024
test "$(sed -n '/^CAUSAL_DIAGNOSTIC /=' "$work/results/enginefail.engine.log")" -lt \
  "$(sed -n '/^invalid parameter token$/=' "$work/results/enginefail.engine.log")"
! grep -q '^CAUSAL_DIAGNOSTIC ' "$work/results/ok.engine.log"
grep -q '"causal_diagnostic": "CAUSAL_DIAGNOSTIC schema=1' "$work/results/rows/enginefail.json"

run_census
test "$(cat "$work/count")" -eq 3

FAKE_UMBER_COUNT="$work/count" \
UMBER_ARXIV_SAMPLE="$work/sample.tsv" \
UMBER_ARXIV_CORPUS="$work/corpus" \
UMBER_ARXIV_ARCHIVES="$work/archives" \
UMBER_ARXIV_FORMAT="$work/format.fmt" \
UMBER_ARXIV_DISTRIBUTION="$work/distribution" \
UMBER_ARXIV_BINARY="$work/fake-umber" \
UMBER_ARXIV_RESULTS="$work/results" \
UMBER_ARXIV_TEXMF="$work/texmf" \
UMBER_ARXIV_LIMIT=3 UMBER_ARXIV_TIMEOUT_SECONDS=10 \
UMBER_ARXIV_MAX_RSS_MIB=128 UMBER_ARXIV_ENGINE_FUEL=1000 \
UMBER_ARXIV_OFFLINE=1 UMBER_ARXIV_VERIFY_ONLY=1 \
"$root/scripts/run-stepwise-arxiv-census.sh"
test "$(cat "$work/count")" -eq 3
test -f "$work/results/offline-verification.json"
