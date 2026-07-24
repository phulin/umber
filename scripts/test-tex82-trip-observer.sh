#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

trip_root="${repo_root}/third_party/trip"
texmfcnf="${repo_root}/third_party/texlive-source/src/texk/web2c/triptrap"
target_dir="${CARGO_TARGET_DIR:-target}"
[[ "$target_dir" == /* ]] || target_dir="${repo_root}/${target_dir}"
oracle_bin="${target_dir}/tex82-oracle/bin"

[[ -f "${trip_root}/trip.tex" && -f "${trip_root}/trip.tfm" ]] || {
  printf 'test-tex82-trip-observer: missing pinned TRIP inputs; run scripts/fetch-conformance-inputs.sh\n' >&2
  exit 1
}

scripts/build-tex82-oracle.sh --offline >/dev/null

work_root="$(mktemp -d "${TMPDIR:-/tmp}/umber-tex82-trip-observer.XXXXXX")"
trap 'rm -rf "$work_root"' EXIT

run_phase() {
  local executable="$1" directory="$2" prefix="$3"
  cp "${trip_root}/trip.tex" "${trip_root}/trip.tfm" "$directory/"
  (
    cd "$directory"
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH=1783604160 FORCE_SOURCE_DATE=1 TEXMFCNF="$texmfcnf" \
      "$executable" -ini -interaction=nonstopmode trip.tex >"${prefix}-initex-terminal.txt" 2>&1
    printf '%s\n' "$?" >"${prefix}-initex-status.txt"
    cp tex82-events.jsonl "${prefix}-initex-events.jsonl" 2>/dev/null || :
    env -i PATH=/usr/bin:/bin LC_ALL=C LANGUAGE=C \
      SOURCE_DATE_EPOCH=1783604160 FORCE_SOURCE_DATE=1 TEXMFCNF="$texmfcnf" \
      "$executable" -interaction=nonstopmode '&trip' trip.tex >"${prefix}-trip-terminal.txt" 2>&1
    printf '%s\n' "$?" >"${prefix}-trip-status.txt"
    cp tex82-events.jsonl "${prefix}-trip-events.jsonl" 2>/dev/null || :
  ) || true
}

normalize_output() {
  sed -E 's/preloaded format=[^)]*/preloaded format=<FORMAT>/' "$1"
}

mkdir -p "$work_root/clean" "$work_root/profile-a" "$work_root/profile-b"
run_phase "${oracle_bin}/umber-tex82-oracle" "$work_root/clean" clean
run_phase "${oracle_bin}/umber-tex82-oracle-trip-profile" "$work_root/profile-a" profile
run_phase "${oracle_bin}/umber-tex82-oracle-trip-profile" "$work_root/profile-b" profile

for profile in profile-a profile-b; do
  cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
    --tex82-trip-profile "$work_root/$profile/profile-initex-events.jsonl"
  cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
    --tex82-trip-profile "$work_root/$profile/profile-trip-events.jsonl"
  cmp "$work_root/clean/trip.dvi" "$work_root/$profile/trip.dvi"
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
cmp "$work_root/profile-a/profile-trip-events.jsonl" "$work_root/profile-b/profile-trip-events.jsonl"
printf 'TeX82 bounded TRIP observer passed\n'
