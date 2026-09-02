#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/umber-pdftex-pdf-survey-test.XXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM

mkdir -p "$work/archives" "$work/runtime/web2c" "$work/results" "$work/report"
printf 'runtime input\n' >"$work/runtime/web2c/texmf.cnf"
runtime_sha=$(sha256sum "$work/runtime/web2c/texmf.cnf" | awk '{print $1}')
printf 'distribution test-runtime\ntree_ahash64 0123456789abcdef\nsource web2c/texmf.cnf 14 %s\n' \
  "$runtime_sha" >"$work/runtime.lock"
printf 'format\n' >"$work/pdflatex.fmt"

cat >"$work/fake-pdftex" <<'EOF'
#!/bin/sh
set -eu
: "${FAKE_PDFTEX_COUNT:?}"
count=0
test ! -f "$FAKE_PDFTEX_COUNT" || count=$(cat "$FAKE_PDFTEX_COUNT")
printf '%s\n' "$((count + 1))" >"$FAKE_PDFTEX_COUNT"
for argument in "$@"; do
  case "$argument" in
    --jobname*) echo 'jobname override is forbidden' >&2; exit 90 ;;
  esac
done
input=$argument
jobname=${input##*/}
jobname=${jobname%.tex}
test -f "$input"
test -f normal-side-file.bbl
printf 'generated side file\n' >"$jobname.aux"
case "$jobname" in
  fails)
    printf '! LaTeX Error: File `missing.sty\x27 not found.\n\n' >"$jobname.log"
    exit 1
    ;;
esac
printf 'pdf\n' >"$jobname.pdf"
printf 'Output written on %s.pdf (1 page, 4 bytes).\n' "$jobname" >"$jobname.log"
EOF
chmod +x "$work/fake-pdftex"
oracle_sha=$(sha256sum "$work/fake-pdftex" | awk '{print $1}')
printf 'executable clean target/pdftex14029-oracle/bin/umber-pdftex14029-oracle-clean %s\n' \
  "$oracle_sha" >"$work/build-record.txt"
format_sha=$(sha256sum "$work/pdflatex.fmt" | awk '{print $1}')
printf '{"engine":{"sha256":"%s"},"format":{"sha256":"%s"}}\n' \
  "$oracle_sha" "$format_sha" >"$work/format-receipt.json"

printf 'id\tsource_sha256\tsource_bytes\tfirst_submitted\tshuffle_sha256\tentrypoint\n' \
  >"$work/source.lock.tsv"
for specification in 'ok:pdflatex:paper.tex' 'fails:pdflatex:fails.tex' \
  'nested:pdflatex:subdir/article.tex' 'latex-only:latex:main.tex'; do
  paper=${specification%%:*}
  remainder=${specification#*:}
  compiler=${remainder%%:*}
  entrypoint=${remainder#*:}
  source="$work/source-$paper"
  mkdir -p "$source/$(dirname "$entrypoint")"
  printf '{"process":{"compiler":"%s"}}\n' "$compiler" >"$source/00README.json"
  printf '\\documentclass{article}\n' >"$source/$entrypoint"
  printf 'side file\n' >"$source/normal-side-file.bbl"
  tar -czf "$work/archives/$paper.src" -C "$source" \
    00README.json normal-side-file.bbl "$entrypoint"
  archive_sha=$(sha256sum "$work/archives/$paper.src" | awk '{print $1}')
  archive_bytes=$(wc -c <"$work/archives/$paper.src" | tr -d ' ')
  printf '%s\t%s\t%s\t2026-01-01\t%s\t%s\n' \
    "$paper" "$archive_sha" "$archive_bytes" "$archive_sha" "$entrypoint" \
    >>"$work/source.lock.tsv"
done

run_survey() {
  FAKE_PDFTEX_COUNT="$work/count" \
    "$root/scripts/survey-pdftex-arxiv-pdf.py" \
    --source-lock "$work/source.lock.tsv" \
    --archives "$work/archives" \
    --oracle "$work/fake-pdftex" \
    --oracle-build-record "$work/build-record.txt" \
    --format "$work/pdflatex.fmt" \
    --format-receipt "$work/format-receipt.json" \
    --runtime-root "$work/runtime" \
    --runtime-lock "$work/runtime.lock" \
    --results "$work/results" \
    --expected-sample-rows 4 \
    --expected-pdflatex-rows 3 \
    --timeout-seconds 10 \
    --max-rss-mib 128 \
    --workers 2 "$@"
}

run_survey
test "$(cat "$work/count")" -eq 3
test ! -e "$work/results/rows/ok/source/paper.aux"
test -f "$work/results/rows/ok/run/paper.aux"
test -f "$work/results/rows/nested/run/article.pdf"
test "$(wc -l <"$work/results/results.jsonl")" -eq 3
grep -q '"PDF-success": 2' "$work/results/summary.json"
grep -q '"PDF-failure": 1' "$work/results/summary.json"
grep -q '"jobname":"article"' "$work/results/results.jsonl"
! grep -q -- '--jobname' "$work/results/results.jsonl"

run_survey
test "$(cat "$work/count")" -eq 3
run_survey --verify-only --report-dir "$work/report"
test "$(cat "$work/count")" -eq 3
test -f "$work/results/verification.json"
test -f "$work/report/results.jsonl"
grep -q '"compilers_launched": 0' "$work/report/verification.json"

echo 'pdfTeX arXiv PDF survey test: PASS'
