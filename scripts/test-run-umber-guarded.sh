#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
guard="$root/scripts/run-umber-guarded.py"
marker="$root/target/guard-self-test-child"
mkdir -p "$root/target"
rm -f "$marker"

set +e
python3 "$guard" --timeout-seconds 1 --max-rss-mib 128 --term-grace-seconds 0.2 -- \
  sh -c 'sh -c '\''trap "" TERM; sleep 60'\'' & echo $! > "$1"; wait' sh "$marker"
status=$?
set -e

test "$status" -eq 124

set +e
UMBER_TRIP_TIMEOUT_SECONDS=10 \
UMBER_TRIP_MAX_RSS_MIB=128 \
UMBER_TRIP_PROGRESS_TIMEOUT_SECONDS=1 \
UMBER_TRIP_TERM_GRACE_SECONDS=0.2 \
  "$root/scripts/trip.sh" sh -c 'sleep 60'
status=$?
set -e
test "$status" -eq 124

progress="$root/target/guard-self-test-progress"
: > "$progress"
set +e
python3 "$guard" --timeout-seconds 10 --max-rss-mib 128 \
  --progress-file "$progress" --progress-timeout-seconds 1 \
  --term-grace-seconds 0.2 -- sh -c 'sleep 60'
status=$?
set -e
test "$status" -eq 124
rm -f "$progress"
child=$(cat "$marker")
if kill -0 "$child" 2>/dev/null; then
  echo "guard self-test: descendant $child survived" >&2
  exit 1
fi
rm -f "$marker"

set +e
python3 "$guard" --timeout-seconds 10 --max-rss-mib 64 --term-grace-seconds 0.2 -- \
  sh -c 'python3 -c "$1" & python3 -c "$1" & wait' sh \
  'import time; allocation = bytearray(32 * 1024 * 1024); time.sleep(60)'
status=$?
set -e

test "$status" -eq 124
