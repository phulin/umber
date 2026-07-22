#!/usr/bin/env bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
guard="$root/scripts/run-umber-guarded.py"
progress="$root/target/trip-watchdog.progress"
timeout=${UMBER_TRIP_TIMEOUT_SECONDS:-120}
rss=${UMBER_TRIP_MAX_RSS_MIB:-6144}
progress_timeout=${UMBER_TRIP_PROGRESS_TIMEOUT_SECONDS:-30}
grace=${UMBER_TRIP_TERM_GRACE_SECONDS:-5}

mkdir -p "$root/target"
: > "$progress"

if (($# == 0)); then
  set -- cargo test -q -p umber --test it e2e_conformance_ -- --nocapture
fi

export UMBER_ENGINE_FUEL=${UMBER_ENGINE_FUEL:-100000000}
export UMBER_TRIP_PROGRESS_FILE="$progress"
python3 "$guard" \
  --timeout-seconds "$timeout" \
  --max-rss-mib "$rss" \
  --progress-file "$progress" \
  --progress-timeout-seconds "$progress_timeout" \
  --term-grace-seconds "$grace" \
  -- bash -o pipefail -c '"$@" 2>&1 | tee -a "$UMBER_TRIP_PROGRESS_FILE"' bash "$@"
