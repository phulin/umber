#!/bin/bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
sample=${UMBER_ARXIV_SAMPLE:-$root/scripts/pdftex-arxiv-sample-100.tsv}
corpus=${UMBER_ARXIV_CORPUS:-$root/third_party/arxiv-sample-100/sources}
format=${UMBER_ARXIV_FORMAT:-}
binary=${UMBER_ARXIV_BINARY:-$root/target/debug/umber}
results=${UMBER_ARXIV_RESULTS:-$root/target/stepwise-arxiv-census}
limit=${UMBER_ARXIV_LIMIT:-100}
timeout=${UMBER_ARXIV_TIMEOUT_SECONDS:-30}
rss=${UMBER_ARXIV_MAX_RSS_MIB:-1536}
fuel=${UMBER_ARXIV_ENGINE_FUEL:-100000000}
texmf=${UMBER_ARXIV_TEXMF:-$root/third_party/texlive-20260301-texmf/texmf-dist}
guard=$root/scripts/run-umber-guarded.py

if [[ -z $format || ! -f $format ]]; then
  echo "UMBER_ARXIV_FORMAT must name a validated pdflatex format image" >&2
  exit 2
fi
if [[ ! -x $binary ]]; then
  echo "Umber binary not found at $binary; run cargo build -q -p umber first" >&2
  exit 2
fi
if [[ ! $limit =~ ^[1-9][0-9]*$ || $limit -gt 100 ]]; then
  echo "UMBER_ARXIV_LIMIT must be in 1..100" >&2
  exit 2
fi

entrypoint() {
  local directory=$1 candidate
  for candidate in main.tex manuscript.tex arxiv_version.tex paper.tex ms.tex; do
    if [[ -f $directory/$candidate ]] && rg -q -F '\documentclass' "$directory/$candidate"; then
      printf '%s\n' "$directory/$candidate"
      return
    fi
  done
  rg -l -F '\documentclass' "$directory" -g '*.tex' \
    | rg -v '/(supp|supplement|appendix)[^/]*\.tex$' \
    | LC_ALL=C sort \
    | head -1
}

mkdir -p "$results"
summary=$results/summary.tsv
printf 'id\tengine_status\tfinalizer_status\tcold_starts\tsuspensions\tlocal_step_retries\treplayed_delivered_tokens\treplayed_dispatches\tcumulative_fuel\tresource_wait_ns\tengine_ns\tguard_status\n' >"$summary"

rows=0
while IFS=$'\t' read -r id _category; do
  [[ $id == id ]] && continue
  ((rows += 1))
  ((rows <= limit)) || break
  key=${id//\//_}
  source_dir=$corpus/$key
  log=$results/$key.engine.log
  main=$(entrypoint "$source_dir" || true)
  if [[ -z $main ]]; then
    printf '%s\tno-entrypoint\tnot-run\t0\t0\t0\t0\t0\t0\t0\t0\t0\n' "$id" >>"$summary"
    continue
  fi

  set +e
  UMBER_RESOURCE_TELEMETRY=1 UMBER_ENGINE_FUEL=$fuel \
    TEXINPUTS="$(dirname "$main"):$texmf/tex/latex//:$texmf/tex/generic//:$texmf/tex/plain//:" \
    TEXFONTS="$texmf/fonts/tfm//:" \
    python3 "$guard" --timeout-seconds "$timeout" --max-rss-mib "$rss" \
      --term-grace-seconds 2 -- "$binary" run --pdflatex --offline --format "$format" "$main" \
      >"$log" 2>&1
  status=$?
  set -e

  telemetry=$(rg '^RESOURCE_TELEMETRY ' "$log" | tail -1 || true)
  value() {
    local name=$1
    sed -n "s/.* $name=\\([0-9][0-9]*\\).*/\\1/p" <<<"$telemetry"
  }
  cold=$(value cold_starts); cold=${cold:-0}
  suspensions=$(value suspensions); suspensions=${suspensions:-0}
  retries=$(value local_step_retries); retries=${retries:-0}
  tokens=$(value replayed_delivered_tokens); tokens=${tokens:-0}
  dispatches=$(value replayed_dispatches); dispatches=${dispatches:-0}
  cumulative=$(value cumulative_fuel); cumulative=${cumulative:-0}
  wait_ns=$(value resource_wait_ns); wait_ns=${wait_ns:-0}
  engine_ns=$(value engine_ns); engine_ns=${engine_ns:-0}
  engine_status=failed
  [[ $status -eq 0 ]] && engine_status=accepted
  [[ $status -eq 124 ]] && engine_status=guard-timeout-or-rss

  finalizer_status=not-run
  if [[ ${UMBER_ARXIV_FINALIZE:-0} == 1 && $engine_status == accepted ]]; then
    pdf=$results/$key.pdf
    final_log=$results/$key.finalizer.log
    set +e
    TEXINPUTS="$(dirname "$main"):$texmf/tex/latex//:$texmf/tex/generic//:$texmf/tex/plain//:" \
      TEXFONTS="$texmf/fonts/tfm//:" \
      python3 "$guard" --timeout-seconds "$timeout" --max-rss-mib "$rss" \
        --term-grace-seconds 2 -- "$binary" run --pdflatex --offline --format "$format" \
        --pdf "$pdf" "$main" >"$final_log" 2>&1
    final_status=$?
    set -e
    finalizer_status=failed
    [[ $final_status -eq 0 ]] && finalizer_status=complete
    [[ $final_status -eq 124 ]] && finalizer_status=guard-timeout-or-rss
  fi

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$id" "$engine_status" "$finalizer_status" "$cold" "$suspensions" "$retries" \
    "$tokens" "$dispatches" "$cumulative" "$wait_ns" "$engine_ns" "$status" >>"$summary"
done <"$sample"

echo "stepwise arXiv census: $summary"
