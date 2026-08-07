#!/bin/sh

# Atomically publish a sealed TRIP observer artifact. A fixed staging name is
# intentional: an interrupted publisher may leave it behind, and the next run
# deterministically replaces that private partial file before publication.
trip_publish_artifact() {
  source_path=$1
  destination=$2
  staging="${destination}.publishing"

  mkdir -p "$(dirname "$destination")"
  rm -f -- "$staging"
  if ! cp -- "$source_path" "$staging"; then
    rm -f -- "$staging"
    return 1
  fi
  if ! chmod 0444 "$staging"; then
    rm -f -- "$staging"
    return 1
  fi
  if ! mv -f -- "$staging" "$destination"; then
    rm -f -- "$staging"
    return 1
  fi
}

# Keep the guarded entry point informed while a cold reference build performs
# long stretches of otherwise silent work. The heartbeat is only observation;
# the wrapped command's status remains authoritative.
trip_run_with_progress() {
  progress_label=$1
  shift
  heartbeat_seconds=${UMBER_TRIP_HEARTBEAT_SECONDS:-60}

  "$@" &
  command_pid=$!
  (
    while kill -0 "$command_pid" 2>/dev/null; do
      printf '%s\n' "$progress_label"
      sleep "$heartbeat_seconds"
    done
  ) &
  heartbeat_pid=$!

  if wait "$command_pid"; then
    command_status=0
  else
    command_status=$?
  fi
  kill "$heartbeat_pid" 2>/dev/null || :
  wait "$heartbeat_pid" 2>/dev/null || :
  return "$command_status"
}
