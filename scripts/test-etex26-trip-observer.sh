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
oracle_bin="${target_dir}/etex26-oracle/bin"

[[ -f "${trip_root}/etrip.tex" && -f "${trip_root}/trip.tfm" &&
  -f "${trip_root}/tripos.tex" ]] || {
  printf 'test-etex26-trip-observer: missing pinned e-TRIP inputs; run python3 scripts/provision.py worktree .\n' >&2
  exit 1
}
trip_run_with_progress 'e-TeX 2.6 oracle build/provision still running' \
  python3 scripts/provision.py oracle etex26 --offline
work_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-etex26-trip-observer.XXXXXX")"
trap 'rm -rf "$work_root"' EXIT

stage_source() {
  local directory="$1"
  {
    printf '%s\n' \
      '% Local e-TeX 2.6 compatibility adaptation; the official etrip.tex remains unchanged.' \
      '% Renamed and modified as required by the e-TeX distribution terms.'
    sed 's/\\def\\etripversion{2.0}/\\def\\etripversion{2.6}/' \
      "${trip_root}/etrip.tex"
  } >"${directory}/etrip.tex"
  cp "${trip_root}/trip.tfm" "${directory}/etrip.tfm"
  cp "${trip_root}/tripos.tex" "${directory}/tripos.tex"
}

run_phase() {
  local executable="$1" directory="$2" prefix="$3"
  stage_source "$directory"
  (
    cd "$directory"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH=1783604160 FORCE_SOURCE_DATE=1 TEXMFCNF="$texmfcnf" \
      "$executable" -ini -interaction=nonstopmode '*etrip.tex' \
      >"${prefix}-initex-terminal.txt" 2>&1
    printf '%s\n' "$?" >"${prefix}-initex-status.txt"
    cp etex26-events.jsonl "${prefix}-initex-events.jsonl" 2>/dev/null || :
    cp etrip.log "${prefix}-initex.log"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH=1783604160 FORCE_SOURCE_DATE=1 TEXMFCNF="$texmfcnf" \
      "$executable" -interaction=nonstopmode '&etrip' etrip.tex \
      >"${prefix}-trip-terminal.txt" 2>&1
    printf '%s\n' "$?" >"${prefix}-trip-status.txt"
    cp etex26-events.jsonl "${prefix}-trip-events.jsonl" 2>/dev/null || :
  ) || true
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

mkdir -p "$work_root/clean" "$work_root/profile-a" "$work_root/profile-b" \
  "$work_root/geometry-a" "$work_root/geometry-b"
run_phase "${oracle_bin}/umber-etex26-extended-oracle-clean" "$work_root/clean" clean
run_phase "${oracle_bin}/umber-etex26-extended-oracle-instrumented" \
  "$work_root/profile-a" profile
run_phase "${oracle_bin}/umber-etex26-extended-oracle-instrumented" \
  "$work_root/profile-b" profile
run_phase "${oracle_bin}/umber-etex26-extended-oracle-trip-geometry-profile" \
  "$work_root/geometry-a" geometry
run_phase "${oracle_bin}/umber-etex26-extended-oracle-trip-geometry-profile" \
  "$work_root/geometry-b" geometry

for phase in initex trip; do
  project_root_session \
    "$work_root/profile-a/profile-${phase}-events.jsonl" \
    "$work_root/profile-a/profile-${phase}-projected.jsonl"
  project_root_session \
    "$work_root/profile-b/profile-${phase}-events.jsonl" \
    "$work_root/profile-b/profile-${phase}-projected.jsonl"
  cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
    "$work_root/profile-a/profile-${phase}-projected.jsonl"
  cmp "$work_root/profile-a/profile-${phase}-projected.jsonl" \
    "$work_root/profile-b/profile-${phase}-projected.jsonl"
  project_geometry \
    "$work_root/geometry-a/geometry-${phase}-events.jsonl" \
    "$work_root/geometry-a/geometry-${phase}-projected.jsonl"
  project_geometry \
    "$work_root/geometry-b/geometry-${phase}-events.jsonl" \
    "$work_root/geometry-b/geometry-${phase}-projected.jsonl"
  cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
    "$work_root/geometry-a/geometry-${phase}-projected.jsonl"
  grep -Fq '"event":"geometry"' \
    "$work_root/geometry-a/geometry-${phase}-projected.jsonl"
  cmp "$work_root/geometry-a/geometry-${phase}-projected.jsonl" \
    "$work_root/geometry-b/geometry-${phase}-projected.jsonl"
done
for phase in initex trip; do
  cmp "$work_root/clean/clean-${phase}-status.txt" \
    "$work_root/profile-a/profile-${phase}-status.txt"
done
cmp "$work_root/clean/etrip.dvi" "$work_root/profile-a/etrip.dvi"

artifact_root="$(trip_observer_artifact_root "$target_dir" etrip)"
mkdir -p "$artifact_root"
trip_publish_artifact "$work_root/profile-a/profile-initex-projected.jsonl" \
  "$artifact_root/initex-command.jsonl"
trip_publish_artifact "$work_root/profile-a/profile-trip-projected.jsonl" \
  "$artifact_root/format-loaded-command.jsonl"
trip_publish_artifact "$work_root/geometry-a/geometry-initex-projected.jsonl" \
  "$artifact_root/initex-geometry.jsonl"
trip_publish_artifact "$work_root/geometry-a/geometry-trip-projected.jsonl" \
  "$artifact_root/format-loaded-geometry.jsonl"
trip_publish_artifact "$work_root/clean/clean-initex-terminal.txt" \
  "$artifact_root/initex-terminal.txt"
trip_publish_artifact "$work_root/clean/clean-initex.log" \
  "$artifact_root/initex.log"
trip_publish_artifact "$work_root/clean/clean-trip-terminal.txt" \
  "$artifact_root/format-loaded-terminal.txt"
trip_publish_artifact "$work_root/clean/etrip.log" \
  "$artifact_root/format-loaded.log"
trip_publish_artifact "$work_root/clean/etrip.dvi" \
  "$artifact_root/format-loaded.dvi"
trip_publish_artifact "${target_dir}/etex26-oracle/build-record.txt" \
  "$artifact_root/oracle-build-record.txt"
printf 'e-TeX 2.6 bounded e-TRIP observer passed\n'
