#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
. scripts/trip-observer-common.sh

trip_root="${repo_root}/third_party/trip"
source_root="${repo_root}/third_party/texlive-source"
texmfcnf="${source_root}/src/texk/web2c/triptrap"
target_dir="${CARGO_TARGET_DIR:-target}"
[[ "$target_dir" == /* ]] || target_dir="${repo_root}/${target_dir}"
oracle_bin="${target_dir}/tex82-oracle/bin"
geometry_only=0

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --geometry-only) geometry_only=1 ;;
    --help|-h)
      printf '%s\n' 'usage: scripts/test-tex82-trip-observer.sh [--geometry-only]'
      exit 0
      ;;
    *)
      printf 'test-tex82-trip-observer: unknown option: %s\n' "$1" >&2
      exit 2
      ;;
  esac
  shift
done

[[ -f "${trip_root}/trip.tex" && -f "${trip_root}/trip.tfm" &&
  -f "${trip_root}/tripos.tex" ]] || {
  printf 'test-tex82-trip-observer: missing pinned TRIP inputs; run python3 scripts/provision.py worktree .\n' >&2
  exit 1
}

trip_run_with_progress 'TeX82 oracle build/provision still running' \
  python3 scripts/provision.py oracle tex82 --offline

work_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-tex82-trip-observer.XXXXXX")"
trap 'rm -rf "$work_root"' EXIT

run_phase() {
  local executable="$1" directory="$2" prefix="$3"
  cp "${trip_root}/trip.tex" "${trip_root}/trip.tfm" \
    "${trip_root}/tripos.tex" "$directory/"
  chmod u+w "$directory/tripos.tex"
  (
    cd "$directory"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH=1783604160 FORCE_SOURCE_DATE=1 TEXMFCNF="$texmfcnf" \
      "$executable" -ini -interaction=nonstopmode trip.tex >"${prefix}-initex-terminal.txt" 2>&1
    printf '%s\n' "$?" >"${prefix}-initex-status.txt"
    cp tex82-events.jsonl "${prefix}-initex-events.jsonl" 2>/dev/null || :
    cp trip.log "${prefix}-initex.log"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH=1783604160 FORCE_SOURCE_DATE=1 TEXMFCNF="$texmfcnf" \
      "$executable" -interaction=nonstopmode '&trip' trip.tex >"${prefix}-trip-terminal.txt" 2>&1
    printf '%s\n' "$?" >"${prefix}-trip-status.txt"
    cp tex82-events.jsonl "${prefix}-trip-events.jsonl" 2>/dev/null || :
  ) || true
}

run_full_initex() {
  local directory="$1"
  cp "${trip_root}/trip.tex" "${trip_root}/trip.tfm" \
    "${trip_root}/tripos.tex" "$directory/"
  chmod u+w "$directory/tripos.tex"
  (
    cd "$directory"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH=1783604160 FORCE_SOURCE_DATE=1 TEXMFCNF="$texmfcnf" \
      "${oracle_bin}/umber-tex82-oracle-instrumentable" \
      -ini -interaction=nonstopmode trip.tex >terminal.txt 2>&1
  ) || true
}

normalize_output() {
  sed -E 's/preloaded format=[^)]*/preloaded format=<FORMAT>/' "$1"
}

dvi_byte_at() {
  od -An -v -tu1 -j "$2" -N 1 "$1" | tr -d '[:space:]'
}

compare_normalized_dvi() {
  local clean_dvi="$1" profile_dvi="$2"
  local clean_comment_len profile_comment_len comment_end
  local clean_size profile_size

  [[ "$(dvi_byte_at "$clean_dvi" 0)" == 247 ]] || {
    printf 'invalid clean DVI preamble: %s\n' "$clean_dvi" >&2
    return 1
  }
  [[ "$(dvi_byte_at "$profile_dvi" 0)" == 247 ]] || {
    printf 'invalid profiled DVI preamble: %s\n' "$profile_dvi" >&2
    return 1
  }
  clean_comment_len="$(dvi_byte_at "$clean_dvi" 14)"
  profile_comment_len="$(dvi_byte_at "$profile_dvi" 14)"
  [[ -n "$clean_comment_len" && "$clean_comment_len" == "$profile_comment_len" ]] || {
    printf 'DVI preamble comment lengths differ\n' >&2
    return 1
  }
  comment_end=$((15 + clean_comment_len))
  clean_size="$(wc -c <"$clean_dvi" | tr -d '[:space:]')"
  profile_size="$(wc -c <"$profile_dvi" | tr -d '[:space:]')"
  ((comment_end <= clean_size && comment_end <= profile_size)) || {
    printf 'truncated DVI preamble comment\n' >&2
    return 1
  }

  # The DVI banner is the sole permitted output normalization. This compares
  # the complete preamble through its length byte, skips only its payload, and
  # then compares every remaining byte (including the postamble and pointers).
  cmp -n 15 "$clean_dvi" "$profile_dvi"
  cmp -s \
    <(dd if="$clean_dvi" bs=1 skip="$comment_end" status=none) \
    <(dd if="$profile_dvi" bs=1 skip="$comment_end" status=none)
}

project_geometry() {
  local input="$1" output="$2"
  {
    sed -n '1p' "$input"
    awk '
      /"event":"geometry"/ {
        sub(/"sequence":[0-9]+/, "\"sequence\":" (sequence + 0))
        print
        sequence++
      }
    ' "$input"
  } >"$output"
}

project_root_session() {
  local input="$1" output="$2"
  {
    sed -n '1p' "$input"
    awk '
      started {
        sub(/"sequence":[0-9]+/, "\"sequence\":" (sequence + 0))
        print
        sequence++
      }
      /"event":"input".*"transition":"push".*"reason":"source"/ { started=1 }
    ' "$input"
  } >"$output"
}

validate_and_publish_geometry() {
  local phase artifact_root event_count expected_events
  for phase in initex trip; do
    project_geometry \
      "$work_root/geometry-a/geometry-${phase}-events.jsonl" \
      "$work_root/geometry-a/geometry-${phase}-projected.jsonl"
    project_geometry \
      "$work_root/geometry-b/geometry-${phase}-events.jsonl" \
      "$work_root/geometry-b/geometry-${phase}-projected.jsonl"
    cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
      "$work_root/geometry-a/geometry-${phase}-projected.jsonl"
    grep -Fq '"event":"geometry"' \
      "$work_root/geometry-a/geometry-${phase}-projected.jsonl" || {
        printf 'TeX82 TRIP %s geometry stream is empty\n' "$phase" >&2
        exit 1
      }
    event_count="$(grep -Fc '"event":"geometry"' \
      "$work_root/geometry-a/geometry-${phase}-projected.jsonl")"
    case "$phase" in
      initex) expected_events=8 ;;
      trip) expected_events=432 ;;
    esac
    [[ "$event_count" -eq "$expected_events" ]] || {
      printf 'TeX82 TRIP %s geometry stream has %s events, expected %s\n' \
        "$phase" "$event_count" "$expected_events" >&2
      exit 1
    }
    cmp "$work_root/geometry-a/geometry-${phase}-projected.jsonl" \
      "$work_root/geometry-b/geometry-${phase}-projected.jsonl"
  done

  artifact_root="${target_dir}/trip-oracles/trip"
  mkdir -p "$artifact_root"
  trip_publish_artifact "$work_root/geometry-a/geometry-initex-projected.jsonl" \
    "$artifact_root/initex-geometry.jsonl"
  trip_publish_artifact "$work_root/geometry-a/geometry-trip-projected.jsonl" \
    "$artifact_root/format-loaded-geometry.jsonl"
  trip_publish_artifact "${target_dir}/tex82-oracle/build-record.txt" \
    "$artifact_root/oracle-build-record.txt"
}

mkdir -p "$work_root/clean" "$work_root/profile-a" "$work_root/profile-b" \
  "$work_root/geometry-a" "$work_root/geometry-b" \
  "$work_root/full-initex-a" "$work_root/full-initex-b"
if [[ "$geometry_only" -eq 1 ]]; then
  python3 scripts/provision.py worktree . --target-dir "$target_dir"
  run_phase "${oracle_bin}/umber-tex82-oracle-trip-geometry-profile" \
    "$work_root/geometry-a" geometry
  run_phase "${oracle_bin}/umber-tex82-oracle-trip-geometry-profile" \
    "$work_root/geometry-b" geometry
  validate_and_publish_geometry
  printf 'TeX82 bounded TRIP geometry observer passed\n'
  exit 0
fi
run_phase "${oracle_bin}/umber-tex82-oracle" "$work_root/clean" clean
run_phase "${oracle_bin}/umber-tex82-oracle-trip-profile" "$work_root/profile-a" profile
run_phase "${oracle_bin}/umber-tex82-oracle-trip-profile" "$work_root/profile-b" profile
run_phase "${oracle_bin}/umber-tex82-oracle-trip-geometry-profile" \
  "$work_root/geometry-a" geometry
run_phase "${oracle_bin}/umber-tex82-oracle-trip-geometry-profile" \
  "$work_root/geometry-b" geometry
run_full_initex "$work_root/full-initex-a"
run_full_initex "$work_root/full-initex-b"
cmp "$work_root/full-initex-a/tex82-events.jsonl" \
  "$work_root/full-initex-b/tex82-events.jsonl"
cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
  "$work_root/full-initex-a/tex82-events.jsonl"
project_root_session "$work_root/full-initex-a/tex82-events.jsonl" \
  "$work_root/full-initex-a/root-session.jsonl"
project_root_session "$work_root/full-initex-b/tex82-events.jsonl" \
  "$work_root/full-initex-b/root-session.jsonl"
cmp "$work_root/full-initex-a/root-session.jsonl" \
  "$work_root/full-initex-b/root-session.jsonl"

for profile in profile-a profile-b; do
  cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
    --tex82-trip-profile "$work_root/$profile/profile-initex-events.jsonl"
  cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
    --tex82-trip-profile "$work_root/$profile/profile-trip-events.jsonl"
  compare_normalized_dvi "$work_root/clean/trip.dvi" "$work_root/$profile/trip.dvi"
  cmp "$work_root/clean/clean-initex-status.txt" "$work_root/$profile/profile-initex-status.txt"
  cmp "$work_root/clean/clean-trip-status.txt" "$work_root/$profile/profile-trip-status.txt"
  for channel in initex-terminal.txt trip-terminal.txt trip.log; do
    if [[ "$channel" == trip.log ]]; then
      clean_channel="$work_root/clean/trip.log"
      profile_channel="$work_root/$profile/trip.log"
    else
      clean_channel="$work_root/clean/clean-$channel"
      profile_channel="$work_root/$profile/profile-$channel"
    fi
    normalize_output "$clean_channel" >"$work_root/clean-$channel"
    normalize_output "$profile_channel" >"$work_root/$profile-$channel"
    cmp "$work_root/clean-$channel" "$work_root/$profile-$channel"
  done
done
cmp "$work_root/profile-a/profile-initex-events.jsonl" "$work_root/profile-b/profile-initex-events.jsonl"
for profile in profile-a profile-b; do
  scripts/project-tex82-trip-command.py \
    "$work_root/$profile/profile-trip-events.jsonl" \
    "$work_root/$profile/profile-trip-command.jsonl"
done
cmp "$work_root/profile-a/profile-trip-command.jsonl" \
  "$work_root/profile-b/profile-trip-command.jsonl"
validate_and_publish_geometry

artifact_root="${target_dir}/trip-oracles/trip"
mkdir -p "$artifact_root"
trip_publish_artifact "$work_root/full-initex-a/root-session.jsonl" \
  "$artifact_root/initex-command.jsonl"
trip_publish_artifact "$work_root/profile-a/profile-trip-command.jsonl" \
  "$artifact_root/format-loaded-command.jsonl"
trip_publish_artifact "$work_root/clean/clean-initex-terminal.txt" \
  "$artifact_root/initex-terminal.txt"
trip_publish_artifact "$work_root/clean/clean-initex.log" \
  "$artifact_root/initex.log"
trip_publish_artifact "$work_root/clean/clean-trip-terminal.txt" \
  "$artifact_root/format-loaded-terminal.txt"
trip_publish_artifact "$work_root/clean/trip.log" \
  "$artifact_root/format-loaded.log"
trip_publish_artifact "$work_root/clean/trip.dvi" \
  "$artifact_root/format-loaded.dvi"
trip_publish_artifact "${target_dir}/tex82-oracle/build-record.txt" \
  "$artifact_root/oracle-build-record.txt"
printf 'TeX82 bounded TRIP observer passed\n'
