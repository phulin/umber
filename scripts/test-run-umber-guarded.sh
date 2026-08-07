#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
guard="$root/scripts/run-umber-guarded.py"
trip_common="$root/scripts/trip-observer-common.sh"
marker="$root/target/guard-self-test-child"

file_sha256() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

mkdir -p "$root/target"
rm -f "$marker"

set +e
python3 "$guard" --timeout-seconds 1 --max-rss-mib 128 --term-grace-seconds 0.2 -- \
  sh -c 'sh -c '\''trap "" TERM; sleep 60'\'' & echo $! > "$1"; wait' sh "$marker"
status=$?
set -e

test "$status" -eq 124

# A completed publication is sealed, but both it and a partial staging file
# left by an interrupted publisher must be replaceable without interaction.
publication_root=$(mktemp -d "${TMPDIR:-/tmp}/umber-trip-publication.XXXXXX")
publication_source="$publication_root/source"
publication_destination="$publication_root/artifact"
printf '%s\n' current >"$publication_source"
printf '%s\n' previous >"$publication_destination"
chmod 0444 "$publication_destination"
set +e
python3 "$guard" --timeout-seconds 10 --max-rss-mib 128 \
  --term-grace-seconds 0.2 -- sh -c \
  'printf "%s\n" interrupted >"${1}.publishing"; chmod 0444 "${1}.publishing"; sleep 60' \
  sh "$publication_destination"
status=$?
set -e
test "$status" -eq 124
. "$trip_common"
trip_publish_artifact "$publication_source" "$publication_destination"
cmp "$publication_source" "$publication_destination"
test ! -e "${publication_destination}.publishing"
test "$(LC_ALL=C ls -ld "$publication_destination" | cut -c 2-10)" = "r--r--r--"
rm -rf "$publication_root"

# Observer publication has a generated namespace distinct from the immutable
# provisioned inputs consumed by the conformance harness. Publishing twice
# must replace the generated artifact without changing locked bytes or modes.
ownership_root=$(mktemp -d "${TMPDIR:-/tmp}/umber-trip-ownership.XXXXXX")
locked_root="$ownership_root/target/trip-oracles/trip"
mkdir -p "$locked_root"
locked_artifact="$locked_root/initex-command.jsonl"
printf '%s\n' provisioned >"$locked_artifact"
chmod 0444 "$locked_artifact"
locked_digest=$(file_sha256 "$locked_artifact")
generated_root=$(trip_observer_artifact_root "$ownership_root/target" trip)
publication_source="$ownership_root/first"
printf '%s\n' first >"$publication_source"
trip_publish_artifact "$publication_source" "$generated_root/initex-command.jsonl"
printf '%s\n' second >"$publication_source"
trip_publish_artifact "$publication_source" "$generated_root/initex-command.jsonl"
test "$(file_sha256 "$locked_artifact")" = "$locked_digest"
test "$(LC_ALL=C ls -ld "$locked_artifact" | cut -c 2-10)" = "r--r--r--"
printf '%s\n' second | cmp - "$generated_root/initex-command.jsonl"
test "$(LC_ALL=C ls -ld "$generated_root/initex-command.jsonl" | cut -c 2-10)" = "r--r--r--"
rm -rf "$ownership_root"

# The oracle builder may be silent for longer than the progress ceiling. Its
# heartbeat must keep the unchanged guard alive until that command completes.
heartbeat_progress="$root/target/guard-self-test-heartbeat-progress"
: > "$heartbeat_progress"
UMBER_TRIP_HEARTBEAT_SECONDS=0.2 \
  python3 "$guard" --timeout-seconds 10 --max-rss-mib 128 \
  --progress-file "$heartbeat_progress" --progress-timeout-seconds 1 \
  --term-grace-seconds 0.2 -- sh -c \
  '. "$1"; trip_run_with_progress "oracle build heartbeat" sh -c "sleep 2" >>"$2"' \
  sh "$trip_common" "$heartbeat_progress"
rm -f "$heartbeat_progress"

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
